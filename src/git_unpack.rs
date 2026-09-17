//! Bounded-memory reader for Git packs received from a remote.
//!
//! A pack is a compressed, deltified copy of a repository's history. Inflating
//! all of it at once costs several times the pack's size in RAM — a 590 MB
//! pack peaked near 6 GB — which is what made `ivaldi download` die on small
//! machines. This module never holds more than a few objects per thread:
//!
//! 1. [`SidebandReader`] demultiplexes the upload-pack response as it
//!    arrives, so the raw response is never buffered.
//! 2. [`index_pack`] walks the pack stream once, *while it downloads*,
//!    recording where each entry lives and validating every zlib stream. The
//!    bytes are teed into a spool file rather than kept in memory.
//! 3. [`PackIndex::resolve`] then rebuilds objects one delta tree at a time
//!    (in parallel across trees), handing each object to a sink and freeing
//!    it as soon as its last delta child has been produced. Objects waiting
//!    on a slow sink are capped by a byte budget.
//!
//! Commits, trees, and tags are small and are collected in memory at fetch
//! time; blobs stay in the spool ([`SpooledPack`]) until import streams them
//! straight into the CAS.

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use flate2::bufread::ZlibDecoder;
use rayon::prelude::*;

use crate::git_remote::{
    GitObject, GitObjectKind, GitRemoteError, apply_delta, git_object_sha1, parse_object_header,
    parse_ofs_delta_base,
};

/// Read buffer for the streaming index pass and for spool reads.
const IO_BUF: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Errors carried through `io::Read`
// ---------------------------------------------------------------------------

/// A protocol-level failure raised from inside a `Read` impl. Travels as the
/// payload of an `io::Error` and is unwrapped back into
/// [`GitRemoteError::Protocol`] by [`stream_error`], so the message survives
/// the trip through `BufReader` and the zlib decoder verbatim.
#[derive(Debug)]
struct ProtocolFailure(String);

impl std::fmt::Display for ProtocolFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProtocolFailure {}

fn protocol_io(message: impl Into<String>) -> io::Error {
    io::Error::other(ProtocolFailure(message.into()))
}

/// Map an error from the pack stream: our own protocol failures verbatim,
/// malformed zlib as a protocol error, anything else as transport I/O.
fn stream_error(err: io::Error) -> GitRemoteError {
    if let Some(failure) = err
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<ProtocolFailure>())
    {
        return GitRemoteError::Protocol(failure.0.clone());
    }
    match err.kind() {
        io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput => {
            GitRemoteError::Protocol(format!("zlib decode failed: {}", err))
        }
        io::ErrorKind::UnexpectedEof => GitRemoteError::Protocol("truncated packfile".into()),
        _ => GitRemoteError::Io(err.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Sideband demultiplexer
// ---------------------------------------------------------------------------

/// Streams the packfile out of a `git-upload-pack` response.
///
/// The response is a run of pkt-lines: negotiation chatter (`NAK`, `ACK`),
/// the optional shallow-update block, then the pack split across
/// `side-band-64k` frames. Reading from this yields only the pack bytes;
/// shallow boundaries are collected on the side.
pub(crate) struct SidebandReader<R> {
    inner: R,
    frame: Vec<u8>,
    pos: usize,
    finished: bool,
    /// Shallow boundaries declared by the server (empty unless we deepened).
    pub shallow: BTreeSet<String>,
    /// Pack bytes handed out so far.
    pub pack_bytes: u64,
    /// Raw response bytes consumed so far.
    pub raw_bytes: u64,
    progress: Option<indicatif::ProgressBar>,
}

impl<R: Read> SidebandReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            frame: Vec::new(),
            pos: 0,
            finished: false,
            shallow: BTreeSet::new(),
            pack_bytes: 0,
            raw_bytes: 0,
            progress: None,
        }
    }

    /// Advance `pb` by every raw response byte consumed.
    pub fn with_progress(mut self, pb: indicatif::ProgressBar) -> Self {
        self.progress = Some(pb);
        self
    }

    /// Read one pkt-line length prefix. `None` at a clean end of stream.
    fn read_length(&mut self) -> io::Result<Option<usize>> {
        let mut prefix = [0u8; 4];
        let mut filled = 0usize;
        while filled < prefix.len() {
            match self.inner.read(&mut prefix[filled..]) {
                Ok(0) if filled == 0 => return Ok(None),
                Ok(0) => return Err(protocol_io("truncated pkt-line")),
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        let text =
            std::str::from_utf8(&prefix).map_err(|_| protocol_io("invalid pkt-line length"))?;
        let len =
            usize::from_str_radix(text, 16).map_err(|_| protocol_io("invalid pkt-line length"))?;
        Ok(Some(len))
    }

    /// Load the next frame carrying pack bytes into `self.frame`. Returns
    /// `false` once the response is exhausted.
    fn next_pack_frame(&mut self) -> io::Result<bool> {
        loop {
            let Some(len) = self.read_length()? else {
                return Ok(false);
            };
            self.advance_progress(4);
            if len == 0 {
                continue; // flush-pkt
            }
            if len < 4 {
                return Err(protocol_io("truncated pkt-line"));
            }
            self.frame.resize(len - 4, 0);
            self.inner.read_exact(&mut self.frame).map_err(|e| {
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    protocol_io("truncated pkt-line")
                } else {
                    e
                }
            })?;
            self.advance_progress(self.frame.len() as u64);

            let line = &self.frame;
            if line == b"NAK\n" || line.starts_with(b"ACK ") {
                continue;
            }
            // Shallow-update block, sent ahead of the pack when we asked to
            // `deepen`. `unshallow` only appears when deepening an existing
            // shallow repo, which `download` never does (it clones into an
            // empty directory), so the boundary set is purely additive here.
            if let Some(sha) = line.strip_prefix(b"shallow ".as_slice()) {
                self.shallow
                    .insert(String::from_utf8_lossy(sha).trim().to_string());
                continue;
            }
            if line.starts_with(b"PACK") {
                self.pos = 0;
                return Ok(true);
            }
            match line.first() {
                Some(1) => {
                    self.pos = 1;
                    if self.frame.len() > 1 {
                        return Ok(true);
                    }
                }
                Some(3) => {
                    let msg = String::from_utf8_lossy(&line[1..]).trim().to_string();
                    return Err(protocol_io(format!("remote error: {}", msg)));
                }
                // Band 2 is progress chatter; anything else is ignorable.
                _ => {}
            }
        }
    }

    fn advance_progress(&mut self, n: u64) {
        self.raw_bytes += n;
        if let Some(pb) = &self.progress {
            pb.inc(n);
        }
    }
}

impl<R: Read> Read for SidebandReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.pos >= self.frame.len() {
            if self.finished {
                return Ok(0);
            }
            if !self.next_pack_frame()? {
                self.finished = true;
                self.frame.clear();
                self.pos = 0;
                return Ok(0);
            }
        }
        let n = buf.len().min(self.frame.len() - self.pos);
        buf[..n].copy_from_slice(&self.frame[self.pos..self.pos + n]);
        self.pos += n;
        self.pack_bytes += n as u64;
        Ok(n)
    }
}

/// Copies everything read through it into `sink` (the spool file).
struct TeeReader<R, W> {
    inner: R,
    sink: W,
}

impl<R: Read, W: Write> Read for TeeReader<R, W> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.sink.write_all(&buf[..n])?;
        Ok(n)
    }
}

/// Tracks how many bytes the parser has consumed, i.e. the current pack
/// offset. `bufread::ZlibDecoder` consumes exactly one zlib stream from a
/// `BufRead`, which is what makes entry boundaries discoverable.
struct CountingReader<R> {
    inner: R,
    consumed: u64,
}

impl<R: BufRead> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.consumed += n as u64;
        Ok(n)
    }
}

impl<R: BufRead> BufRead for CountingReader<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.consumed += amt as u64;
        self.inner.consume(amt);
    }
}

// ---------------------------------------------------------------------------
// Random access to a stored pack
// ---------------------------------------------------------------------------

/// Positional reads that don't disturb a shared cursor, so every resolver
/// thread can read the same pack concurrently.
pub(crate) trait ReadAt: Sync {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize>;
}

impl ReadAt for [u8] {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(self.len());
        let n = buf.len().min(self.len() - start);
        buf[..n].copy_from_slice(&self[start..start + n]);
        Ok(n)
    }
}

impl ReadAt for File {
    #[cfg(unix)]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        std::os::unix::fs::FileExt::read_at(self, buf, offset)
    }

    #[cfg(windows)]
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        std::os::windows::fs::FileExt::seek_read(self, buf, offset)
    }
}

/// Sequential `Read` over a [`ReadAt`] source, starting at `pos`.
struct SourceCursor<'a, S: ?Sized> {
    source: &'a S,
    pos: u64,
}

impl<S: ReadAt + ?Sized> Read for SourceCursor<'_, S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.source.read_at(buf, self.pos)?;
        self.pos += n as u64;
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// Index pass
// ---------------------------------------------------------------------------

enum EntryKind {
    Base(GitObjectKind),
    /// The base is recorded from its side, in `PackIndex::ofs_children`.
    OfsDelta,
    RefDelta([u8; 20]),
}

struct IndexEntry {
    /// Where the entry's header starts; `ofs-delta` bases are named by this.
    offset: u64,
    /// Where the entry's zlib stream starts.
    data_offset: u64,
    /// Inflated size, verified against the stream during indexing.
    size: usize,
    kind: EntryKind,
}

/// Where every entry of a pack lives, plus the delta forest connecting them.
/// Holds no object data.
pub(crate) struct PackIndex {
    entries: Vec<IndexEntry>,
    /// `ofs-delta` children of each entry, by entry index.
    ofs_children: Vec<Vec<u32>>,
    /// `ref-delta` children, keyed by the base object's id.
    ref_children: HashMap<[u8; 20], Vec<u32>>,
    /// Set once an entry has been rebuilt and handed to a sink.
    resolved: Vec<AtomicBool>,
}

impl std::fmt::Debug for PackIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackIndex")
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// Read a varint-style run: bytes up to and including the first without the
/// continuation bit, capped at `out.len()`. The slice parsers then apply
/// their own overflow rules to it.
fn read_continued<R: BufRead>(reader: &mut R, out: &mut [u8]) -> Result<usize, GitRemoteError> {
    let mut used = 0usize;
    while used < out.len() {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).map_err(stream_error)?;
        out[used] = byte[0];
        used += 1;
        if byte[0] & 0x80 == 0 {
            break;
        }
    }
    Ok(used)
}

/// Index a pack from a stream, validating every entry on the way.
///
/// Network-facing: every count, size, and varint in the pack is validated
/// before it drives an allocation, and each object's zlib stream is capped at
/// its declared size. `total_len`, when the whole pack is already in hand,
/// lets a forged entry count be refused up front.
///
/// The reader is drained to its end so a tee underneath captures the whole
/// pack, trailer included.
pub(crate) fn index_pack<R: BufRead>(
    reader: R,
    total_len: Option<u64>,
) -> Result<PackIndex, GitRemoteError> {
    let mut reader = CountingReader {
        inner: reader,
        consumed: 0,
    };

    let mut header = [0u8; 12];
    reader.read_exact(&mut header).map_err(|e| match e.kind() {
        io::ErrorKind::UnexpectedEof => GitRemoteError::Protocol("invalid packfile header".into()),
        _ => stream_error(e),
    })?;
    if &header[..4] != b"PACK" {
        return Err(GitRemoteError::Protocol("invalid packfile header".into()));
    }
    let version = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
    if !(2..=3).contains(&version) {
        return Err(GitRemoteError::Unsupported(format!(
            "unsupported pack version {}",
            version
        )));
    }
    let count = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
    // The smallest possible entry is a 1-byte header plus an ~8-byte zlib
    // stream; a claimed count beyond that is corrupt. A streamed pack has no
    // known length, so there the count is simply never used to pre-allocate.
    if let Some(len) = total_len
        && count as u64 > len / 9
    {
        return Err(GitRemoteError::Protocol(format!(
            "corrupt packfile: claims {} entries but is only {} bytes",
            count, len
        )));
    }

    let mut entries: Vec<IndexEntry> = Vec::new();
    // (entry index, encoded base offset) — resolved to indices once every
    // entry offset is known.
    let mut pending_ofs: Vec<(u32, u64)> = Vec::new();
    let mut scratch = [0u8; 16];

    for i in 0..count {
        let offset = reader.consumed;
        let used = read_continued(&mut reader, &mut scratch)?;
        let (kind, _, size) = parse_object_header(&scratch[..used])?;

        let kind = match kind {
            1 => EntryKind::Base(GitObjectKind::Commit),
            2 => EntryKind::Base(GitObjectKind::Tree),
            3 => EntryKind::Base(GitObjectKind::Blob),
            4 => EntryKind::Base(GitObjectKind::Tag),
            6 => {
                let used = read_continued(&mut reader, &mut scratch)?;
                let here = usize::try_from(offset).map_err(|_| {
                    GitRemoteError::Protocol("invalid ofs-delta base offset".into())
                })?;
                let (base_offset, _) = parse_ofs_delta_base(&scratch[..used], here)?;
                pending_ofs.push((i as u32, base_offset as u64));
                EntryKind::OfsDelta
            }
            7 => {
                let mut base = [0u8; 20];
                reader.read_exact(&mut base).map_err(|e| match e.kind() {
                    io::ErrorKind::UnexpectedEof => {
                        GitRemoteError::Protocol("truncated ref-delta base".into())
                    }
                    _ => stream_error(e),
                })?;
                EntryKind::RefDelta(base)
            }
            other => {
                return Err(GitRemoteError::Unsupported(format!(
                    "unsupported object type {}",
                    other
                )));
            }
        };

        let data_offset = reader.consumed;
        // Inflate to nowhere: this pass only needs the stream's end and the
        // proof that it produces exactly the declared size. +1 so an
        // over-long stream is detectable rather than silently clipped.
        let mut limited = ZlibDecoder::new(&mut reader).take(size as u64 + 1);
        let inflated = io::copy(&mut limited, &mut io::sink()).map_err(stream_error)?;
        if inflated != size as u64 {
            return Err(GitRemoteError::Protocol(format!(
                "object inflated to {}+ bytes but its header declared {}",
                inflated, size
            )));
        }

        entries.push(IndexEntry {
            offset,
            data_offset,
            size,
            kind,
        });
    }

    // Drain the trailer (and any post-pack frames) so the spool is complete
    // and a late `remote error` frame still surfaces.
    io::copy(&mut reader, &mut io::sink()).map_err(stream_error)?;

    let mut ofs_children: Vec<Vec<u32>> = (0..entries.len()).map(|_| Vec::new()).collect();
    for (child, base_offset) in pending_ofs {
        // Entries are in offset order, so the base is a binary search away.
        let base = entries
            .binary_search_by_key(&base_offset, |e| e.offset)
            .map_err(|_| GitRemoteError::Protocol("unresolvable delta chain in packfile".into()))?;
        ofs_children[base].push(child);
    }
    let mut ref_children: HashMap<[u8; 20], Vec<u32>> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        if let EntryKind::RefDelta(base) = entry.kind {
            ref_children.entry(base).or_default().push(i as u32);
        }
    }
    let resolved = (0..entries.len()).map(|_| AtomicBool::new(false)).collect();

    Ok(PackIndex {
        entries,
        ofs_children,
        ref_children,
        resolved,
    })
}

// ---------------------------------------------------------------------------
// Resolve pass
// ---------------------------------------------------------------------------

/// Cap on object bytes queued for a sink on other threads. Past it, the
/// thread that rebuilt an object delivers it itself, which is what keeps a
/// fast delta walk from piling a whole chain up in memory behind slow sinks.
const MAX_QUEUED_BYTES: usize = 128 << 20;

/// A rebuilt object whose delta children are still to be produced.
struct Frame {
    data: Arc<Vec<u8>>,
    children: Vec<u32>,
}

/// Shared state of one [`PackIndex::resolve`] run.
struct Resolver<'a, S: ?Sized, F> {
    index: &'a PackIndex,
    source: &'a S,
    sink: &'a F,
    queued_bytes: AtomicUsize,
    failed: AtomicBool,
    failure: Mutex<Option<GitRemoteError>>,
}

impl PackIndex {
    /// Inflate one entry's payload. Sizes were verified by [`index_pack`], so
    /// the declared size is safe to pre-allocate here.
    fn inflate<S: ReadAt + ?Sized>(&self, source: &S, idx: u32) -> Result<Vec<u8>, GitRemoteError> {
        let entry = &self.entries[idx as usize];
        let cursor = SourceCursor {
            source,
            pos: entry.data_offset,
        };
        let reader = BufReader::with_capacity(IO_BUF.min(entry.size + 1024), cursor);
        let mut out = Vec::with_capacity(entry.size);
        ZlibDecoder::new(reader)
            .take(entry.size as u64 + 1)
            .read_to_end(&mut out)
            .map_err(stream_error)?;
        if out.len() != entry.size {
            return Err(GitRemoteError::Protocol(format!(
                "object inflated to {}+ bytes but its header declared {}",
                out.len(),
                entry.size
            )));
        }
        Ok(out)
    }

    /// Rebuild every object descending from a base entry whose kind passes
    /// `want`, handing each to `sink` as `(kind, sha1 hex, data)`.
    ///
    /// Delta trees are independent, so they resolve in parallel; within a
    /// tree the walk is depth-first and a parent is freed as soon as its last
    /// child has been derived. Peak memory is therefore a few objects per
    /// thread rather than the whole repository.
    ///
    /// A delta chain can only be walked serially, so hashing and `sink` are
    /// pushed onto other threads where possible: a long chain of large blobs
    /// then costs one delta application per link on the walking thread
    /// instead of a full hash-and-store. `sink` runs on arbitrary threads, in
    /// no particular order.
    pub(crate) fn resolve<S, W, F>(
        &self,
        source: &S,
        want: W,
        sink: F,
    ) -> Result<(), GitRemoteError>
    where
        S: ReadAt + ?Sized,
        W: Fn(GitObjectKind) -> bool,
        F: Fn(GitObjectKind, String, Arc<Vec<u8>>) -> Result<(), GitRemoteError> + Sync,
    {
        let roots: Vec<(u32, GitObjectKind)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| match e.kind {
                EntryKind::Base(kind) if want(kind) => Some((i as u32, kind)),
                _ => None,
            })
            .collect();

        let resolver = Resolver {
            index: self,
            source,
            sink: &sink,
            queued_bytes: AtomicUsize::new(0),
            failed: AtomicBool::new(false),
            failure: Mutex::new(None),
        };
        rayon::scope(|scope| {
            roots.par_iter().for_each(|&(root, kind)| {
                if resolver.failed.load(Ordering::Relaxed) {
                    return;
                }
                if let Err(e) = resolver.resolve_tree(scope, root, kind) {
                    resolver.fail(e);
                }
            });
        });
        match resolver.failure.into_inner().unwrap() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Fail if any entry was never rebuilt — a delta whose base is not in the
    /// pack. Only meaningful once every kind has been through [`resolve`].
    ///
    /// [`resolve`]: PackIndex::resolve
    pub(crate) fn ensure_fully_resolved(&self) -> Result<(), GitRemoteError> {
        if self.resolved.iter().all(|r| r.load(Ordering::Relaxed)) {
            Ok(())
        } else {
            Err(GitRemoteError::Protocol(
                "unresolvable delta chain in packfile".into(),
            ))
        }
    }
}

impl<'a, S, F> Resolver<'a, S, F>
where
    S: ReadAt + ?Sized,
    F: Fn(GitObjectKind, String, Arc<Vec<u8>>) -> Result<(), GitRemoteError> + Sync,
{
    /// Record the first failure; later ones are consequences of the abort.
    fn fail(&self, err: GitRemoteError) {
        self.failed.store(true, Ordering::Relaxed);
        self.failure.lock().unwrap().get_or_insert(err);
    }

    /// Iterative on purpose: chain depth is attacker-controlled.
    fn resolve_tree<'scope>(
        &'scope self,
        scope: &rayon::Scope<'scope>,
        root: u32,
        kind: GitObjectKind,
    ) -> Result<(), GitRemoteError> {
        let index = self.index;
        if index.resolved[root as usize].swap(true, Ordering::Relaxed) {
            return Ok(());
        }
        let mut stack: Vec<Frame> = Vec::new();
        let data = index.inflate(self.source, root)?;
        self.emit(scope, root, kind, data, &mut stack)?;

        while let Some(top) = stack.last_mut() {
            if self.failed.load(Ordering::Relaxed) {
                return Ok(());
            }
            let Some(child) = top.children.pop() else {
                stack.pop();
                continue;
            };
            // A ref-delta can be reachable through duplicate copies of its
            // base; rebuild it once.
            if index.resolved[child as usize].swap(true, Ordering::Relaxed) {
                continue;
            }
            let delta = index.inflate(self.source, child)?;
            let data = apply_delta(&top.data, &delta)?;
            drop(delta);
            if top.children.is_empty() {
                // Release the parent before descending: a linear chain then
                // holds two objects at a time, not the whole chain.
                stack.pop();
            }
            self.emit(scope, child, kind, data, &mut stack)?;
        }
        Ok(())
    }

    /// Deliver a rebuilt object and queue its delta children.
    fn emit<'scope>(
        &'scope self,
        scope: &rayon::Scope<'scope>,
        idx: u32,
        kind: GitObjectKind,
        data: Vec<u8>,
        stack: &mut Vec<Frame>,
    ) -> Result<(), GitRemoteError> {
        let index = self.index;
        let data = Arc::new(data);
        let mut children = index.ofs_children[idx as usize].clone();
        // The walk itself only needs the id to find ref-delta children. Packs
        // fetched with `ofs-delta` have none, and then hashing moves off this
        // thread along with the sink.
        let sha = if index.ref_children.is_empty() {
            None
        } else {
            let sha = git_object_sha1(kind, &data);
            if let Some(by_ref) = index.ref_children.get(&sha) {
                children.extend_from_slice(by_ref);
            }
            Some(sha)
        };
        self.deliver(scope, kind, sha, Arc::clone(&data))?;
        if !children.is_empty() {
            stack.push(Frame { data, children });
        }
        Ok(())
    }

    fn deliver<'scope>(
        &'scope self,
        scope: &rayon::Scope<'scope>,
        kind: GitObjectKind,
        sha: Option<[u8; 20]>,
        data: Arc<Vec<u8>>,
    ) -> Result<(), GitRemoteError> {
        let len = data.len();
        let run = move |this: &Self| {
            let sha = sha.unwrap_or_else(|| git_object_sha1(kind, &data));
            (this.sink)(kind, hex::encode(sha), data)
        };
        if self.queued_bytes.fetch_add(len, Ordering::Relaxed) + len > MAX_QUEUED_BYTES {
            self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
            return run(self);
        }
        scope.spawn(move |_| {
            if !self.failed.load(Ordering::Relaxed)
                && let Err(e) = run(self)
            {
                self.fail(e);
            }
            self.queued_bytes.fetch_sub(len, Ordering::Relaxed);
        });
        Ok(())
    }
}

/// Collect resolved objects into a `sha → object` map.
fn collect_objects<S, W>(
    index: &PackIndex,
    source: &S,
    want: W,
) -> Result<HashMap<String, GitObject>, GitRemoteError>
where
    S: ReadAt + ?Sized,
    W: Fn(GitObjectKind) -> bool,
{
    let objects = Mutex::new(HashMap::new());
    index.resolve(source, want, |kind, sha, data| {
        objects
            .lock()
            .unwrap()
            .entry(sha)
            .or_insert_with(|| GitObject {
                kind,
                data: Arc::try_unwrap(data).unwrap_or_else(|shared| shared.to_vec()),
            });
        Ok(())
    })?;
    Ok(objects.into_inner().unwrap())
}

/// Parse an in-memory pack into a `sha1 → object` map (every kind).
pub(crate) fn parse_pack_bytes(data: &[u8]) -> Result<HashMap<String, GitObject>, GitRemoteError> {
    let index = index_pack(data, Some(data.len() as u64))?;
    let objects = collect_objects(&index, data, |_| true)?;
    index.ensure_fully_resolved()?;
    Ok(objects)
}

// ---------------------------------------------------------------------------
// Spooled pack
// ---------------------------------------------------------------------------

static SPOOL_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The spool file, deleted when dropped.
struct Spool {
    file: Option<File>,
    /// Still-linked path (platforms that can't unlink an open file).
    path: Option<PathBuf>,
}

impl Spool {
    /// The spool lives next to the repository being written rather than in
    /// the system temp directory: `/tmp` is frequently RAM-backed, which
    /// would put the pack right back in memory.
    fn create(dir: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!(
            ".ivaldi-fetch-{}-{}.pack",
            std::process::id(),
            SPOOL_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        // Unlink immediately where the platform allows it: the spool then
        // can't outlive the process, even on SIGKILL.
        let unlinked = cfg!(unix) && std::fs::remove_file(&path).is_ok();
        Ok(Self {
            file: Some(file),
            path: (!unlinked).then_some(path),
        })
    }

    fn file(&self) -> &File {
        self.file.as_ref().expect("spool open until drop")
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        // Close first: Windows refuses to delete a file that is still open.
        self.file.take();
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// A received pack parked on disk, from which blobs are rebuilt on demand.
pub struct SpooledPack {
    spool: Spool,
    index: PackIndex,
}

impl std::fmt::Debug for SpooledPack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpooledPack")
            .field("index", &self.index)
            .finish()
    }
}

/// What [`receive_pack`] extracted from an upload-pack response.
pub(crate) struct ReceivedPack {
    /// Commits, trees, and tags, by sha1 hex.
    pub objects: HashMap<String, GitObject>,
    /// The spooled pack, holding the blobs.
    pub pack: SpooledPack,
    pub shallow: BTreeSet<String>,
    /// Raw response bytes read off the wire.
    pub raw_bytes: u64,
}

/// Consume a `git-upload-pack` response stream: spool the pack under
/// `spool_dir`, index it while it arrives, then load the history objects
/// (commits, trees, tags). Blobs are left in the spool for import.
pub(crate) fn receive_pack<R: Read>(
    response: R,
    spool_dir: &Path,
    progress: indicatif::ProgressBar,
) -> Result<ReceivedPack, GitRemoteError> {
    let spool = Spool::create(spool_dir)
        .map_err(|e| GitRemoteError::Io(format!("creating pack spool: {}", e)))?;

    let mut sideband = SidebandReader::new(response).with_progress(progress);
    let indexed = {
        let mut tee = TeeReader {
            inner: &mut sideband,
            sink: io::BufWriter::with_capacity(IO_BUF, spool.file()),
        };
        index_pack(BufReader::with_capacity(IO_BUF, &mut tee), None).and_then(|index| {
            tee.sink
                .flush()
                .map_err(|e| GitRemoteError::Io(format!("writing pack spool: {}", e)))?;
            Ok(index)
        })
    };
    let index = match indexed {
        Ok(index) => index,
        Err(_) if sideband.pack_bytes < 12 => {
            return Err(GitRemoteError::Protocol(
                "upload-pack response did not contain a packfile".into(),
            ));
        }
        Err(e) => return Err(e),
    };
    let pack = SpooledPack { spool, index };

    let objects = collect_objects(&pack.index, pack.spool.file(), |kind| {
        kind != GitObjectKind::Blob
    })?;

    Ok(ReceivedPack {
        objects,
        pack,
        shallow: sideband.shallow,
        raw_bytes: sideband.raw_bytes,
    })
}

impl SpooledPack {
    /// Rebuild every blob in the pack, handing each to `sink` as
    /// `(sha1 hex, content)` from multiple threads. Errors if the pack turns
    /// out to contain a delta with no base.
    pub(crate) fn for_each_blob<F>(&self, sink: F) -> Result<(), GitRemoteError>
    where
        F: Fn(String, &[u8]) -> Result<(), GitRemoteError> + Sync,
    {
        self.index.resolve(
            self.spool.file(),
            |kind| kind == GitObjectKind::Blob,
            |_, sha, data| sink(sha, &data),
        )?;
        self.index.ensure_fully_resolved()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_remote::{encode_pack_for_tests, git_object_id, pkt_line};
    use flate2::Compression;
    use flate2::write::ZlibEncoder;

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    /// Size varint as used inside delta payloads.
    fn delta_varint(mut n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let mut b = (n & 0x7f) as u8;
            n >>= 7;
            if n > 0 {
                b |= 0x80;
            }
            out.push(b);
            if n == 0 {
                return out;
            }
        }
    }

    /// A delta that ignores its base and inserts `result` literally.
    fn insert_delta(base_len: usize, result: &[u8]) -> Vec<u8> {
        assert!(result.len() < 128);
        let mut d = delta_varint(base_len);
        d.extend(delta_varint(result.len()));
        d.push(result.len() as u8);
        d.extend_from_slice(result);
        d
    }

    fn entry_header(kind: u8, size: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let mut rest = size >> 4;
        let mut first = (kind << 4) | (size & 0x0f) as u8;
        if rest > 0 {
            first |= 0x80;
        }
        out.push(first);
        while rest > 0 {
            let mut b = (rest & 0x7f) as u8;
            rest >>= 7;
            if rest > 0 {
                b |= 0x80;
            }
            out.push(b);
        }
        out
    }

    /// Pack: blob `v1`, an ofs-delta `v2` on it, a ref-delta `v3` on `v2`
    /// placed *before* its base is resolvable, plus one commit-kind base.
    fn delta_pack() -> (Vec<u8>, [Vec<u8>; 3]) {
        let v1 = b"first version of the file\n".to_vec();
        let v2 = b"second version of the file\n".to_vec();
        let v3 = b"third version of the file\n".to_vec();
        let v2_sha = git_object_sha1(GitObjectKind::Blob, &v2);

        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&4u32.to_be_bytes());

        // ref-delta first: its base (v2) only exists once the ofs-delta
        // below has been applied.
        let d3 = insert_delta(v2.len(), &v3);
        pack.extend(entry_header(7, d3.len()));
        pack.extend_from_slice(&v2_sha);
        pack.extend(zlib(&d3));

        let v1_offset = pack.len();
        pack.extend(entry_header(3, v1.len()));
        pack.extend(zlib(&v1));

        let d2 = insert_delta(v1.len(), &v2);
        let d2_offset = pack.len();
        pack.extend(entry_header(6, d2.len()));
        pack.push((d2_offset - v1_offset) as u8); // single-byte ofs varint
        pack.extend(zlib(&d2));

        let commit = b"tree 0000000000000000000000000000000000000000\n\nmsg\n";
        pack.extend(entry_header(1, commit.len()));
        pack.extend(zlib(commit));

        pack.extend_from_slice(&[0u8; 20]); // trailer (not verified)
        (pack, [v1, v2, v3])
    }

    #[test]
    fn resolves_ofs_and_ref_delta_chains() {
        let (pack, versions) = delta_pack();
        let objects = parse_pack_bytes(&pack).unwrap();
        assert_eq!(objects.len(), 4);
        for v in &versions {
            let got = &objects[&git_object_id(GitObjectKind::Blob, v)];
            assert_eq!(got.kind, GitObjectKind::Blob);
            assert_eq!(&got.data, v);
        }
    }

    #[test]
    fn kind_filter_splits_history_from_blobs() {
        let (pack, _) = delta_pack();
        let index = index_pack(pack.as_slice(), Some(pack.len() as u64)).unwrap();
        let history =
            collect_objects(&index, pack.as_slice(), |k| k != GitObjectKind::Blob).unwrap();
        assert_eq!(history.len(), 1);
        // Blobs not resolved yet, so the pack is not fully accounted for.
        assert!(index.ensure_fully_resolved().is_err());
        let blobs = collect_objects(&index, pack.as_slice(), |k| k == GitObjectKind::Blob).unwrap();
        assert_eq!(blobs.len(), 3);
        index.ensure_fully_resolved().unwrap();
    }

    #[test]
    fn ref_delta_with_absent_base_is_refused() {
        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&1u32.to_be_bytes());
        let d = insert_delta(3, b"abc");
        pack.extend(entry_header(7, d.len()));
        pack.extend_from_slice(&[0xab; 20]);
        pack.extend(zlib(&d));
        let err = parse_pack_bytes(&pack).unwrap_err();
        assert!(err.to_string().contains("unresolvable"), "{}", err);
    }

    #[test]
    fn ofs_delta_pointing_between_entries_is_refused() {
        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend(entry_header(3, 3));
        pack.extend(zlib(b"abc"));
        let d = insert_delta(3, b"abd");
        let here = pack.len();
        pack.extend(entry_header(6, d.len()));
        pack.push((here - 13) as u8); // one byte past the base's header
        pack.extend(zlib(&d));
        let err = parse_pack_bytes(&pack).unwrap_err();
        assert!(err.to_string().contains("unresolvable"), "{}", err);
    }

    fn sideband_response(pack: &[u8], frame: usize) -> Vec<u8> {
        let mut resp = pkt_line("NAK\n");
        for chunk in pack.chunks(frame) {
            let mut payload = vec![1u8];
            payload.extend_from_slice(chunk);
            resp.extend(format!("{:04x}", payload.len() + 4).into_bytes());
            resp.extend(payload);
        }
        // Progress chatter mid-stream must be dropped.
        resp.extend(pkt_line("\u{2}counting objects\n"));
        resp.extend_from_slice(b"0000");
        resp
    }

    #[test]
    fn sideband_reader_reassembles_pack_across_frames() {
        let (pack, _) = delta_pack();
        let resp = sideband_response(&pack, 7);
        let mut out = Vec::new();
        let mut reader = SidebandReader::new(resp.as_slice());
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, pack);
        assert_eq!(reader.raw_bytes, resp.len() as u64);
    }

    #[test]
    fn sideband_reader_surfaces_remote_error_frames() {
        let mut resp = pkt_line("NAK\n");
        resp.extend(pkt_line("\u{3}access denied\n"));
        let mut out = Vec::new();
        let err = SidebandReader::new(resp.as_slice())
            .read_to_end(&mut out)
            .unwrap_err();
        let err = stream_error(err);
        assert!(matches!(err, GitRemoteError::Protocol(ref m) if m.contains("access denied")));
    }

    #[test]
    fn sideband_reader_rejects_truncated_frame() {
        let mut resp = pkt_line("NAK\n");
        resp.extend_from_slice(b"0010\x01abc"); // claims 12 payload bytes, has 4
        let mut out = Vec::new();
        let err = SidebandReader::new(resp.as_slice())
            .read_to_end(&mut out)
            .unwrap_err();
        assert!(stream_error(err).to_string().contains("truncated"));
    }

    #[test]
    fn receive_pack_spools_and_streams_blobs() {
        let (pack, versions) = delta_pack();
        let resp = sideband_response(&pack, 11);
        let dir = tempfile::tempdir().unwrap();
        let received = receive_pack(
            resp.as_slice(),
            dir.path(),
            indicatif::ProgressBar::hidden(),
        )
        .unwrap();
        // History is in memory; blobs are not.
        assert_eq!(received.objects.len(), 1);
        assert!(
            received
                .objects
                .values()
                .all(|o| o.kind == GitObjectKind::Commit)
        );

        let seen = Mutex::new(Vec::new());
        received
            .pack
            .for_each_blob(|sha, data| {
                seen.lock().unwrap().push((sha, data.to_vec()));
                Ok(())
            })
            .unwrap();
        let mut seen = seen.into_inner().unwrap();
        seen.sort();
        let mut expected: Vec<(String, Vec<u8>)> = versions
            .iter()
            .map(|v| (git_object_id(GitObjectKind::Blob, v), v.clone()))
            .collect();
        expected.sort();
        assert_eq!(seen, expected);

        drop(received);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "spool must not outlive the fetch"
        );
    }

    fn hex20(sha: &str) -> Vec<u8> {
        hex::decode(sha).unwrap()
    }

    fn tree_with(entries: &[(&str, &str, &str)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (mode, name, sha) in entries {
            data.extend_from_slice(format!("{} {}\0", mode, name).as_bytes());
            data.extend(hex20(sha));
        }
        data
    }

    fn commit_with(tree: &str, parent: Option<&str>) -> Vec<u8> {
        let mut c = format!("tree {}\n", tree);
        if let Some(p) = parent {
            c.push_str(&format!("parent {}\n", p));
        }
        c.push_str("author T <t@x> 1710000000 +0000\n");
        c.push_str("committer T <t@x> 1710000000 +0000\n\nmsg\n");
        c.into_bytes()
    }

    /// Which part of the two-commit test history a response carries.
    #[derive(Clone, Copy, PartialEq)]
    enum Slice {
        /// Everything; the second `a.txt` is an ofs-delta of the first.
        Full,
        /// Commits and trees but no blobs, as a broken server would send.
        NoBlobs,
        /// Only the first commit.
        First,
        /// Only what the second commit adds — what a server sends after
        /// being told `have <first>`: no first commit, no unchanged `sub`
        /// tree, no first blob.
        Second,
    }

    struct History {
        response: Vec<u8>,
        first: String,
        second: String,
        first_tree: String,
        versions: [Vec<u8>; 2],
    }

    /// Two commits: the second adds a nested directory (the first commit's
    /// whole tree, unchanged) and rewrites `a.txt`.
    fn history(slice: Slice) -> History {
        let v1 = b"first version\n".to_vec();
        let v2 = b"second version\n".to_vec();
        let (v1_sha, v2_sha) = (
            git_object_id(GitObjectKind::Blob, &v1),
            git_object_id(GitObjectKind::Blob, &v2),
        );
        let t1 = tree_with(&[("100644", "a.txt", &v1_sha)]);
        let t1_sha = git_object_id(GitObjectKind::Tree, &t1);
        let t2 = tree_with(&[("100644", "a.txt", &v2_sha), ("40000", "sub", &t1_sha)]);
        let t2_sha = git_object_id(GitObjectKind::Tree, &t2);
        let c1 = commit_with(&t1_sha, None);
        let c1_sha = git_object_id(GitObjectKind::Commit, &c1);
        let c2 = commit_with(&t2_sha, Some(&c1_sha));
        let c2_sha = git_object_id(GitObjectKind::Commit, &c2);

        let plain: Vec<(u8, &Vec<u8>)> = match slice {
            Slice::Full | Slice::NoBlobs => vec![(1, &c2), (1, &c1), (2, &t2), (2, &t1)],
            Slice::First => vec![(1, &c1), (2, &t1), (3, &v1)],
            Slice::Second => vec![(1, &c2), (2, &t2), (3, &v2)],
        };
        let count = plain.len() as u32 + if slice == Slice::Full { 2 } else { 0 };
        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&2u32.to_be_bytes());
        pack.extend_from_slice(&count.to_be_bytes());
        for (kind, data) in plain {
            pack.extend(entry_header(kind, data.len()));
            pack.extend(zlib(data));
        }
        if slice == Slice::Full {
            let v1_offset = pack.len();
            pack.extend(entry_header(3, v1.len()));
            pack.extend(zlib(&v1));
            let delta = insert_delta(v1.len(), &v2);
            let here = pack.len();
            pack.extend(entry_header(6, delta.len()));
            pack.push((here - v1_offset) as u8);
            pack.extend(zlib(&delta));
        }
        pack.extend_from_slice(&[0u8; 20]);

        let mut response = sideband_response(&pack, 64);
        if slice == Slice::Second {
            // A negotiated response acknowledges the common commit first.
            let mut acked = pkt_line(&format!("ACK {} common\n", c1_sha));
            acked.extend(pkt_line(&format!("ACK {}\n", c1_sha)));
            acked.extend_from_slice(&response[pkt_line("NAK\n").len()..]);
            response = acked;
        }
        History {
            response,
            first: c1_sha,
            second: c2_sha,
            first_tree: t1_sha,
            versions: [v1, v2],
        }
    }

    fn history_response(with_blobs: bool) -> (Vec<u8>, String, [Vec<u8>; 2]) {
        let h = history(if with_blobs {
            Slice::Full
        } else {
            Slice::NoBlobs
        });
        (h.response, h.second, h.versions)
    }

    fn forged_repo() -> (tempfile::TempDir, crate::repo::Repo) {
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let repo = crate::repo::Repo::open(dir.path()).unwrap();
        (dir, repo)
    }

    fn import(
        repo: &mut crate::repo::Repo,
        response: &[u8],
        head: &str,
    ) -> Result<crate::sync::ImportResult, GitRemoteError> {
        let fetch = fetch_from(response, head, &repo.ivaldi_dir.clone());
        crate::git_remote::import_fetch_result(repo, &fetch)
    }

    fn head_tree(repo: &crate::repo::Repo) -> crate::hash::B3Hash {
        let idx = repo.get_timeline_head("main").unwrap().unwrap();
        repo.get_leaf(idx).unwrap().unwrap().tree_root
    }

    #[test]
    fn negotiated_pack_lands_on_existing_history() {
        let (_full_dir, mut full) = forged_repo();
        let everything = history(Slice::Full);
        import(&mut full, &everything.response, &everything.second).unwrap();

        let (_dir, mut repo) = forged_repo();
        let first = history(Slice::First);
        import(&mut repo, &first.response, &first.first).unwrap();
        let second = history(Slice::Second);
        let result = import(&mut repo, &second.response, &second.second).unwrap();

        assert_eq!(result.commits_imported, 1);
        assert_eq!(result.blobs_downloaded, 1);
        assert_eq!(repo.commit_count(), 2);
        // Same seals as importing everything at once: the unchanged `sub`
        // tree and the parent were resolved from the local store.
        assert_eq!(head_tree(&repo), head_tree(&full));
        let head = repo.get_timeline_head("main").unwrap().unwrap();
        let full_head = full.get_timeline_head("main").unwrap().unwrap();
        assert_eq!(
            repo.get_leaf(head).unwrap().unwrap().hash(),
            full.get_leaf(full_head).unwrap().unwrap().hash()
        );
    }

    #[test]
    fn negotiated_pack_onto_missing_history_names_the_missing_object() {
        let (_dir, mut repo) = forged_repo();
        let second = history(Slice::Second);
        let err = import(&mut repo, &second.response, &second.second).unwrap_err();
        assert!(
            matches!(err, GitRemoteError::MissingObject { kind: "commit", ref sha } if *sha == second.first),
            "{}",
            err
        );
        assert_eq!(repo.commit_count(), 0, "nothing may be sealed");
    }

    #[test]
    fn mapping_entry_without_its_object_is_not_trusted() {
        let (_dir, mut repo) = forged_repo();
        let first = history(Slice::First);
        import(&mut repo, &first.response, &first.first).unwrap();

        // Prune the first tree from the CAS, as gc would after a squash. The
        // mapping still names it.
        let mapping = crate::remote::HashMapping::new(&repo.ivaldi_dir);
        let tree_hash = mapping.get_blake3(&first.first_tree).unwrap();
        let cas = crate::cas::FileCas::new(repo.ivaldi_dir.join("objects")).unwrap();
        assert!(cas.remove(tree_hash).unwrap());

        let second = history(Slice::Second);
        let err = import(&mut repo, &second.response, &second.second).unwrap_err();
        assert!(
            matches!(err, GitRemoteError::MissingObject { kind: "tree", ref sha } if *sha == first.first_tree),
            "{}",
            err
        );
        assert_eq!(repo.commit_count(), 1, "the failed import sealed nothing");
    }

    fn fetch_from(response: &[u8], head: &str, spool: &Path) -> crate::git_remote::FetchResult {
        let received = receive_pack(response, spool, indicatif::ProgressBar::hidden()).unwrap();
        crate::git_remote::FetchResult {
            branch: "main".into(),
            head_sha: head.to_string(),
            refs: Vec::new(),
            objects: received.objects,
            pack: Some(Arc::new(received.pack)),
            shallow: received.shallow,
        }
    }

    #[test]
    fn spooled_fetch_imports_end_to_end() {
        let (response, head, versions) = history_response(true);
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = crate::repo::Repo::open(dir.path()).unwrap();
        let fetch = fetch_from(&response, &head, &repo.ivaldi_dir.clone());
        assert!(
            fetch
                .objects
                .values()
                .all(|o| o.kind != GitObjectKind::Blob),
            "blobs must stay in the spool"
        );

        let result = crate::git_remote::import_fetch_result(&mut repo, &fetch).unwrap();
        assert_eq!(result.commits_imported, 2);
        assert_eq!(result.blobs_downloaded, 2);

        let cas = crate::cas::FileCas::new(repo.ivaldi_dir.join("objects")).unwrap();
        let store = crate::fsmerkle::FsStore::new(&cas);
        let mapping = crate::remote::HashMapping::new(&repo.ivaldi_dir);
        for v in &versions {
            let hash = mapping
                .get_blake3(&git_object_id(GitObjectKind::Blob, v))
                .expect("blob mapped");
            assert_eq!(&store.load_blob(hash).unwrap().1, v);
        }

        // The head seal's tree is the second commit's: new a.txt plus sub/.
        let head_idx = repo.get_timeline_head("main").unwrap().unwrap();
        let leaf = repo.get_leaf(head_idx).unwrap().unwrap();
        let root = store.load_tree(leaf.tree_root).unwrap();
        let names: Vec<&str> = root.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["a.txt", "sub"]);
        let sub = store.load_tree(root.entries[1].hash).unwrap();
        assert_eq!(store.load_blob(sub.entries[0].hash).unwrap().1, versions[0]);
    }

    #[test]
    fn spooled_fetch_missing_a_reachable_blob_is_refused() {
        let (response, head, _) = history_response(false);
        let dir = tempfile::tempdir().unwrap();
        crate::forge::forge(dir.path()).unwrap();
        let mut repo = crate::repo::Repo::open(dir.path()).unwrap();
        let fetch = fetch_from(&response, &head, &repo.ivaldi_dir.clone());
        let err = crate::git_remote::import_fetch_result(&mut repo, &fetch).unwrap_err();
        assert!(err.to_string().contains("missing blob object"), "{}", err);
    }

    #[test]
    fn response_without_a_pack_is_reported_as_such() {
        let resp = pkt_line("NAK\n");
        let dir = tempfile::tempdir().unwrap();
        let err = receive_pack(
            resp.as_slice(),
            dir.path(),
            indicatif::ProgressBar::hidden(),
        )
        .err()
        .unwrap();
        assert!(
            err.to_string().contains("did not contain a packfile"),
            "{}",
            err
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn streamed_index_matches_in_memory_parse() {
        let objects = vec![
            (GitObjectKind::Blob, b"hello\n".to_vec()),
            (GitObjectKind::Blob, vec![7u8; 200_000]),
            (GitObjectKind::Tree, Vec::new()),
        ];
        let pack = encode_pack_for_tests(&objects);
        let parsed = parse_pack_bytes(&pack).unwrap();
        assert_eq!(parsed.len(), 3);
        // A 1-byte BufReader forces every boundary case in the streaming path.
        let index = index_pack(BufReader::with_capacity(1, pack.as_slice()), None).unwrap();
        let streamed = collect_objects(&index, pack.as_slice(), |_| true).unwrap();
        assert_eq!(streamed.len(), 3);
        for (sha, obj) in parsed {
            assert_eq!(streamed[&sha].data, obj.data);
        }
    }
}
