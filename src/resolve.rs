//! Settling the collisions a fuse cannot settle by itself.
//!
//! The engine merges everything that has one right answer: files only one
//! side changed, and — line by line — files both sides changed in different
//! places. What is left is genuinely ambiguous: the same lines rewritten two
//! ways, a binary changed twice, an edit on one side of a deletion on the
//! other. No algorithm knows which was meant.
//!
//! The usual answer is to dump conflict markers into the working files, leave
//! the merge half open, and make the user edit whole files and come back with
//! `--continue`. Here the question is put to a [`Resolver`] instead, one
//! collision at a time, *before anything is written*: the fuse either
//! completes in the one command or changes nothing at all.
//!
//! A resolver is a policy ([`PreferResolver`], for `--prefer` and scripts) or
//! a person ([`PromptResolver`], on a terminal). Picking a side by rule is
//! never the silent default — a rule-picked region can compile and still be
//! wrong — so with neither a policy nor a terminal, the fuse refuses.

use std::io::{BufRead, Write};
use std::path::Path;

use crate::fsmerkle::FsStore;
use crate::fuse::{Chunk, Collision, Conflict, Merge3, Resolution};
use crate::hash::B3Hash;

/// A standing answer for every collision: `--prefer mine|theirs|both`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefer {
    Mine,
    Theirs,
    Both,
}

impl std::str::FromStr for Prefer {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "mine" | "ours" => Ok(Prefer::Mine),
            "theirs" => Ok(Prefer::Theirs),
            "both" => Ok(Prefer::Both),
            other => Err(format!(
                "unknown preference: {other}. Options: mine, theirs, both"
            )),
        }
    }
}

/// Which whole version of a file to keep, when its content cannot be merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Mine,
    Theirs,
}

/// Why a file has to be chosen whole rather than settled region by region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WholeFile {
    /// Changed on both sides, and at least one version is not text.
    Binary,
    /// Mine changed it; theirs deleted it.
    DeletedByTheirs,
    /// Theirs changed it; mine deleted it.
    DeletedByMine,
}

/// Why resolving stopped without an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The user backed out. Nothing has been written.
    Cancelled,
    /// The resolver has no answer for this conflict (`--prefer both` on a
    /// binary). Carries what to tell the user.
    Unresolvable(String),
}

/// Names for the two sides, as the user thinks of them, and what backing out
/// would mean at this point — which differs by caller, and which a person
/// deciding whether to quit is owed the truth about.
#[derive(Debug, Clone, Copy)]
pub struct Labels<'a> {
    pub mine: &'a str,
    pub theirs: &'a str,
    /// Completes the prompt's "quit — …".
    pub on_quit: &'a str,
}

/// One collision, with enough around it to judge.
pub struct Region<'a> {
    pub path: &'a str,
    /// 1-based position among this file's collisions, and how many there are.
    pub index: usize,
    pub total: usize,
    pub collision: &'a Collision,
    /// Merged lines just before and after, for context.
    pub before: &'a [String],
    pub after: &'a [String],
    pub labels: Labels<'a>,
}

pub trait Resolver {
    fn region(&mut self, region: &Region<'_>) -> Result<Resolution, Stop>;
    fn whole_file(&mut self, path: &str, why: WholeFile, labels: Labels<'_>) -> Result<Side, Stop>;
}

/// Settle one engine-reported conflict. `Ok(Some(blob))` is the file's merged
/// content; `Ok(None)` means the file is deleted.
pub fn resolve_conflict(
    store: &FsStore<'_>,
    conflict: &Conflict,
    labels: Labels<'_>,
    resolver: &mut dyn Resolver,
) -> Result<Option<B3Hash>, Stop> {
    let unreadable = |e| Stop::Unresolvable(format!("cannot read {}: {}", conflict.path, e));
    let (ours, theirs) = match (conflict.ours, conflict.theirs) {
        (Some(ours), Some(theirs)) => (ours, theirs),
        (Some(ours), None) => {
            let side = resolver.whole_file(&conflict.path, WholeFile::DeletedByTheirs, labels)?;
            return Ok((side == Side::Mine).then_some(ours));
        }
        (None, Some(theirs)) => {
            let side = resolver.whole_file(&conflict.path, WholeFile::DeletedByMine, labels)?;
            return Ok((side == Side::Theirs).then_some(theirs));
        }
        (None, None) => return Ok(None),
    };

    let base = match conflict.base {
        Some(hash) => store.load_blob(hash).map_err(unreadable)?.1,
        None => Vec::new(),
    };
    let ours_bytes = store.load_blob(ours).map_err(unreadable)?.1;
    let theirs_bytes = store.load_blob(theirs).map_err(unreadable)?.1;

    let text = |bytes: &[u8]| -> Option<String> {
        (!crate::diff::is_binary(bytes))
            .then(|| String::from_utf8(bytes.to_vec()).ok())
            .flatten()
    };
    let (Some(base), Some(ours_text), Some(theirs_text)) =
        (text(&base), text(&ours_bytes), text(&theirs_bytes))
    else {
        return Ok(Some(
            match resolver.whole_file(&conflict.path, WholeFile::Binary, labels)? {
                Side::Mine => ours,
                Side::Theirs => theirs,
            },
        ));
    };

    let merge = Merge3::new(&base, &ours_text, &theirs_text);
    let resolutions = resolve_regions(&merge, &conflict.path, labels, resolver)?;
    let merged = merge.render(&resolutions, labels.mine, labels.theirs);
    let (hash, _) = store
        .put_blob(merged.as_bytes())
        .map_err(|e| Stop::Unresolvable(format!("cannot store {}: {}", conflict.path, e)))?;
    Ok(Some(hash))
}

/// Ask `resolver` about every collision in `merge`, in file order.
pub fn resolve_regions(
    merge: &Merge3,
    path: &str,
    labels: Labels<'_>,
    resolver: &mut dyn Resolver,
) -> Result<Vec<Option<Resolution>>, Stop> {
    let total = merge.collisions();
    let mut resolutions = Vec::with_capacity(total);

    for (collision, before, after) in collisions_in_context(merge) {
        resolutions.push(Some(resolver.region(&Region {
            path,
            index: resolutions.len() + 1,
            total,
            collision,
            before,
            after,
            labels,
        })?));
    }
    Ok(resolutions)
}

/// Every collision in `merge`, in file order, with the merged lines just
/// before and after it for context.
fn collisions_in_context(
    merge: &Merge3,
) -> impl Iterator<Item = (&Collision, &[String], &[String])> {
    const CONTEXT: usize = 3;
    merge.chunks.iter().enumerate().filter_map(|(i, chunk)| {
        let Chunk::Collision(collision) = chunk else {
            return None;
        };
        let before: &[String] = match i.checked_sub(1).map(|j| &merge.chunks[j]) {
            Some(Chunk::Clean(lines)) => &lines[lines.len().saturating_sub(CONTEXT)..],
            _ => &[],
        };
        let after: &[String] = match merge.chunks.get(i + 1) {
            Some(Chunk::Clean(lines)) => &lines[..lines.len().min(CONTEXT)],
            _ => &[],
        };
        Some((collision, before, after))
    })
}

// -- Questions, for front ends that cannot block --------------------------------

/// The same questions a [`Resolver`] is asked, laid out as a list.
///
/// A `Resolver` is a callback: it suits a front end that can stop and wait for
/// an answer. An event loop cannot — it has to draw a question, return, and
/// get the answer as a later event. So it takes the whole list up front, keeps
/// it as state, records answers as they come, and calls [`Questions::apply`]
/// once there are none left.
pub struct Questions {
    files: Vec<FileQuestions>,
}

enum FileQuestions {
    Text {
        path: String,
        merge: Merge3,
        answers: Vec<Option<Resolution>>,
    },
    Whole {
        path: String,
        why: WholeFile,
        ours: Option<B3Hash>,
        theirs: Option<B3Hash>,
        answer: Option<Side>,
    },
}

/// One thing to ask.
pub enum Question<'a> {
    Region {
        path: &'a str,
        /// 1-based position among this file's collisions, and how many.
        index: usize,
        total: usize,
        collision: &'a Collision,
        before: &'a [String],
        after: &'a [String],
    },
    WholeFile {
        path: &'a str,
        why: WholeFile,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    Region(Resolution),
    WholeFile(Side),
}

impl Questions {
    /// Lay out everything there is to ask about `conflicts`.
    pub fn new(store: &FsStore<'_>, conflicts: &[Conflict]) -> Result<Self, String> {
        let text = |hash: Option<B3Hash>| -> Result<Option<String>, String> {
            let bytes = match hash {
                Some(hash) => store.load_blob(hash).map_err(|e| e.to_string())?.1,
                None => Vec::new(),
            };
            Ok((!crate::diff::is_binary(&bytes))
                .then(|| String::from_utf8(bytes).ok())
                .flatten())
        };
        let mut files = Vec::new();
        for c in conflicts {
            let whole = |why| FileQuestions::Whole {
                path: c.path.clone(),
                why,
                ours: c.ours,
                theirs: c.theirs,
                answer: None,
            };
            files.push(match (c.ours, c.theirs) {
                (None, None) => continue,
                (Some(_), None) => whole(WholeFile::DeletedByTheirs),
                (None, Some(_)) => whole(WholeFile::DeletedByMine),
                (Some(_), Some(_)) => match (text(c.base)?, text(c.ours)?, text(c.theirs)?) {
                    (Some(base), Some(ours), Some(theirs)) => {
                        let merge = Merge3::new(&base, &ours, &theirs);
                        let answers = vec![None; merge.collisions()];
                        FileQuestions::Text {
                            path: c.path.clone(),
                            merge,
                            answers,
                        }
                    }
                    _ => whole(WholeFile::Binary),
                },
            });
        }
        Ok(Self { files })
    }

    /// Questions about line merges already worked out — the collisions
    /// between a fuse and uncommitted work ([`crate::carry::preview`]).
    /// Merges without collisions contribute nothing.
    pub fn from_merges(merges: Vec<(String, Merge3)>) -> Self {
        Self {
            files: merges
                .into_iter()
                .filter(|(_, merge)| merge.collisions() > 0)
                .map(|(path, merge)| FileQuestions::Text {
                    answers: vec![None; merge.collisions()],
                    path,
                    merge,
                })
                .collect(),
        }
    }

    /// Put every question to `resolver`, in order, recording the answers —
    /// for a front end that *can* block but wants its answers before acting.
    /// Stops at the first the resolver will not answer.
    pub fn ask(&mut self, resolver: &mut dyn Resolver, labels: Labels<'_>) -> Result<(), Stop> {
        for file in &mut self.files {
            match file {
                FileQuestions::Text {
                    path,
                    merge,
                    answers,
                } => *answers = resolve_regions(merge, path, labels, resolver)?,
                FileQuestions::Whole {
                    path, why, answer, ..
                } => *answer = Some(resolver.whole_file(path, *why, labels)?),
            }
        }
        Ok(())
    }

    /// The answers, as a [`Resolver`] that gives them back when the same
    /// questions are asked again — for answers collected *before* the
    /// operation that will ask.
    pub fn into_resolver(self) -> AnsweredResolver {
        let mut answers = std::collections::BTreeMap::new();
        for file in self.files {
            if let FileQuestions::Text {
                path,
                merge,
                answers: given,
            } = file
            {
                let collisions: Vec<Collision> = merge
                    .chunks
                    .into_iter()
                    .filter_map(|chunk| match chunk {
                        Chunk::Collision(c) => Some(c),
                        Chunk::Clean(_) => None,
                    })
                    .collect();
                answers.insert(path, collisions.into_iter().zip(given).collect());
            }
        }
        AnsweredResolver { answers }
    }

    pub fn len(&self) -> usize {
        self.files
            .iter()
            .map(|f| match f {
                FileQuestions::Text { answers, .. } => answers.len(),
                FileQuestions::Whole { .. } => 1,
            })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Which file question `i` falls in, and its position within that file.
    fn locate(&self, mut i: usize) -> Option<(usize, usize)> {
        for (f, file) in self.files.iter().enumerate() {
            let n = match file {
                FileQuestions::Text { answers, .. } => answers.len(),
                FileQuestions::Whole { .. } => 1,
            };
            if i < n {
                return Some((f, i));
            }
            i -= n;
        }
        None
    }

    pub fn get(&self, i: usize) -> Option<Question<'_>> {
        let (f, within) = self.locate(i)?;
        Some(match &self.files[f] {
            FileQuestions::Text {
                path,
                merge,
                answers,
            } => {
                let (collision, before, after) = collisions_in_context(merge).nth(within)?;
                Question::Region {
                    path,
                    index: within + 1,
                    total: answers.len(),
                    collision,
                    before,
                    after,
                }
            }
            FileQuestions::Whole { path, why, .. } => Question::WholeFile { path, why: *why },
        })
    }

    /// Record an answer. `false` if `i` is out of range or the answer is the
    /// wrong kind for that question — in which case nothing is recorded.
    pub fn answer(&mut self, i: usize, answer: Answer) -> bool {
        let Some((f, within)) = self.locate(i) else {
            return false;
        };
        match (&mut self.files[f], answer) {
            (FileQuestions::Text { answers, .. }, Answer::Region(resolution)) => {
                answers[within] = Some(resolution);
                true
            }
            (FileQuestions::Whole { answer, .. }, Answer::WholeFile(side)) => {
                *answer = Some(side);
                true
            }
            _ => false,
        }
    }

    pub fn answered(&self, i: usize) -> bool {
        match self.locate(i).map(|(f, within)| (&self.files[f], within)) {
            Some((FileQuestions::Text { answers, .. }, within)) => answers[within].is_some(),
            Some((FileQuestions::Whole { answer, .. }, _)) => answer.is_some(),
            None => false,
        }
    }

    /// The first question at or after `from` (wrapping) with no answer yet.
    pub fn next_unanswered(&self, from: usize) -> Option<usize> {
        let n = self.len();
        (0..n).map(|k| (from + k) % n).find(|&i| !self.answered(i))
    }

    /// Write the answers into `merged`: each settled file's content, or its
    /// removal. Refuses unless every question has an answer — a collision
    /// nobody decided must never be sealed as conflict markers.
    pub fn apply(
        &self,
        store: &FsStore<'_>,
        merged: &mut std::collections::BTreeMap<String, B3Hash>,
    ) -> Result<(), String> {
        if let Some(i) = self.next_unanswered(0) {
            return Err(format!(
                "question {} of {} is unanswered",
                i + 1,
                self.len()
            ));
        }
        for file in &self.files {
            match file {
                FileQuestions::Text {
                    path,
                    merge,
                    answers,
                } => {
                    let text = merge.render(answers, "", "");
                    let (hash, _) = store.put_blob(text.as_bytes()).map_err(|e| e.to_string())?;
                    merged.insert(path.clone(), hash);
                }
                FileQuestions::Whole {
                    path,
                    ours,
                    theirs,
                    answer,
                    ..
                } => {
                    let kept = match answer {
                        Some(Side::Mine) => *ours,
                        Some(Side::Theirs) => *theirs,
                        None => unreachable!("checked above"),
                    };
                    match kept {
                        Some(hash) => merged.insert(path.clone(), hash),
                        None => merged.remove(path),
                    };
                }
            }
        }
        Ok(())
    }
}

/// Answers given ahead of time, replayed when the questions are asked.
///
/// An answer is only given back for the *same* collision it was given to: if
/// the files changed between asking and replaying, so the collision there now
/// is a different one, it declines rather than apply an answer to a question
/// nobody was asked.
pub struct AnsweredResolver {
    answers: std::collections::BTreeMap<String, Vec<(Collision, Option<Resolution>)>>,
}

impl AnsweredResolver {
    /// Not [`Stop::Cancelled`]: nobody backed out. Having no answer for this
    /// file says nothing about the next one.
    fn no_answer(path: &str) -> Stop {
        Stop::Unresolvable(format!(
            "{path} changed since its collisions were asked about"
        ))
    }
}

impl Resolver for AnsweredResolver {
    fn region(&mut self, region: &Region<'_>) -> Result<Resolution, Stop> {
        self.answers
            .get(region.path)
            .and_then(|file| file.get(region.index - 1))
            .filter(|(asked, _)| asked == region.collision)
            .and_then(|(_, answer)| answer.clone())
            .ok_or_else(|| Self::no_answer(region.path))
    }

    fn whole_file(&mut self, path: &str, _: WholeFile, _: Labels<'_>) -> Result<Side, Stop> {
        Err(Self::no_answer(path))
    }
}

/// One line per conflict — path and what kind of collision it is — for telling
/// a user what a fuse or sync would have had to ask about.
pub fn describe_conflicts(store: &FsStore<'_>, conflicts: &[Conflict]) -> Vec<String> {
    let text = |hash: Option<B3Hash>| -> Option<String> {
        let bytes = match hash {
            Some(hash) => store.load_blob(hash).ok()?.1,
            None => Vec::new(),
        };
        (!crate::diff::is_binary(&bytes))
            .then(|| String::from_utf8(bytes).ok())
            .flatten()
    };
    conflicts
        .iter()
        .map(|c| {
            let what = match (c.ours, c.theirs) {
                (Some(_), None) => "changed here, deleted there".to_string(),
                (None, Some(_)) => "deleted here, changed there".to_string(),
                _ => match (text(c.base), text(c.ours), text(c.theirs)) {
                    (Some(base), Some(ours), Some(theirs)) => format!(
                        "{} collision(s)",
                        Merge3::new(&base, &ours, &theirs).collisions()
                    ),
                    _ => "binary".to_string(),
                },
            };
            format!("{}  ({})", c.path, what)
        })
        .collect()
}

/// What a caller with collisions on its hands should do about them.
pub enum Collisions<'a> {
    /// Settle them with this resolver, before anything is written.
    Ask(&'a mut (dyn Resolver + 'static)),
    /// Write conflict markers and leave the merge open for `fuse --continue`.
    Markers,
    /// There is nobody to ask and no standing answer: change nothing.
    Refuse,
    /// Nobody to ask *here*, but there will be: change nothing, and keep what
    /// was fetched on its scratch timeline so a front end that can ask — the
    /// TUI's Fuse tab — can fuse it from there without fetching again.
    Park,
}

// -- Policy -------------------------------------------------------------------

/// Answers every collision the same way.
pub struct PreferResolver(pub Prefer);

impl Resolver for PreferResolver {
    fn region(&mut self, _region: &Region<'_>) -> Result<Resolution, Stop> {
        Ok(match self.0 {
            Prefer::Mine => Resolution::Ours,
            Prefer::Theirs => Resolution::Theirs,
            Prefer::Both => Resolution::Both,
        })
    }

    fn whole_file(
        &mut self,
        path: &str,
        why: WholeFile,
        _labels: Labels<'_>,
    ) -> Result<Side, Stop> {
        match self.0 {
            Prefer::Mine => Ok(Side::Mine),
            Prefer::Theirs => Ok(Side::Theirs),
            Prefer::Both => Err(Stop::Unresolvable(format!(
                "{path}: {} — there is no keeping both. Use --prefer mine or --prefer theirs, \
                 or run on a terminal to choose per file.",
                match why {
                    WholeFile::Binary => "binary, changed on both sides",
                    WholeFile::DeletedByTheirs | WholeFile::DeletedByMine =>
                        "changed on one side and deleted on the other",
                }
            ))),
        }
    }
}

// -- Person -------------------------------------------------------------------

/// Opens `path` for the user to edit and returns when they are done.
pub type Editor = Box<dyn FnMut(&Path) -> std::io::Result<()>>;

/// Asks a person, one collision at a time.
///
/// Generic over its streams so the dialogue is testable; on a terminal these
/// are stdin and stdout.
pub struct PromptResolver<R: BufRead, W: Write> {
    input: R,
    out: W,
    editor: Editor,
}

impl<R: BufRead, W: Write> PromptResolver<R, W> {
    pub fn new(input: R, out: W, editor: Editor) -> Self {
        Self { input, out, editor }
    }

    /// One line of input, trimmed and lowercased. End of input is a quit:
    /// a closed stdin must never be read as consent to anything.
    fn answer(&mut self) -> Result<String, Stop> {
        let _ = self.out.flush();
        let mut line = String::new();
        match self.input.read_line(&mut line) {
            Ok(0) | Err(_) => Err(Stop::Cancelled),
            Ok(_) => Ok(line.trim().to_ascii_lowercase()),
        }
    }

    fn show(&mut self, region: &Region<'_>) {
        use crate::color;
        let c = region.collision;
        let o = &mut self.out;
        let _ = writeln!(o);
        let _ = writeln!(
            o,
            "{}",
            color::bold(&format!(
                "{} — collision {} of {}, around line {}",
                region.path, region.index, region.total, c.ours_line
            ))
        );
        for line in region.before {
            let _ = writeln!(o, "    {}", color::dim(line));
        }
        let side = |o: &mut W, name: &str, lines: &[String], paint: fn(&str) -> String| {
            let _ = writeln!(o, "  {}", paint(&format!("{name}:")));
            if lines.is_empty() {
                let _ = writeln!(o, "    {}", color::dim("(these lines deleted)"));
            }
            for line in lines {
                let _ = writeln!(o, "  {} {}", paint("|"), line);
            }
        };
        side(
            o,
            &format!("mine ({})", region.labels.mine),
            &c.ours,
            color::green,
        );
        side(
            o,
            &format!("theirs ({})", region.labels.theirs),
            &c.theirs,
            color::cyan,
        );
        for line in region.after {
            let _ = writeln!(o, "    {}", color::dim(line));
        }
    }

    /// Hand the region to the user's editor with both versions in it, and take
    /// back whatever they leave — unless they leave the markers, which means
    /// they have not decided.
    fn edit(&mut self, region: &Region<'_>) -> Result<Option<Vec<String>>, Stop> {
        let buffer = edit_buffer(
            region.collision,
            region.before,
            region.after,
            region.labels.mine,
            region.labels.theirs,
        );
        let edited = edit_text(&mut self.editor, region.path, &buffer)
            .map_err(|e| Stop::Unresolvable(format!("could not run editor: {e}")))?;
        match parse_edited(&edited, region.before, region.after) {
            Ok(lines) => Ok(Some(lines)),
            Err(why) => {
                let _ = writeln!(self.out, "  {why} Not taken as an answer.");
                Ok(None)
            }
        }
    }
}

impl<R: BufRead, W: Write> Resolver for PromptResolver<R, W> {
    fn region(&mut self, region: &Region<'_>) -> Result<Resolution, Stop> {
        self.show(region);
        loop {
            let _ = write!(
                self.out,
                "  [m]ine  [t]heirs  [b]oth (mine, then theirs)  [e]dit  [q]uit — {} > ",
                region.labels.on_quit
            );
            let picked = match self.answer()?.as_str() {
                "m" | "mine" => Resolution::Ours,
                "t" | "theirs" => Resolution::Theirs,
                "b" | "both" => Resolution::Both,
                "e" | "edit" => match self.edit(region)? {
                    Some(lines) => Resolution::Custom(lines),
                    None => continue,
                },
                "q" | "quit" => return Err(Stop::Cancelled),
                _ => continue,
            };
            return Ok(picked);
        }
    }

    fn whole_file(&mut self, path: &str, why: WholeFile, labels: Labels<'_>) -> Result<Side, Stop> {
        let (mine, theirs) = match why {
            WholeFile::Binary => ("keep my version", "take their version"),
            WholeFile::DeletedByTheirs => ("keep my changed file", "delete it, as they did"),
            WholeFile::DeletedByMine => ("keep it deleted", "take their changed file"),
        };
        let _ = writeln!(self.out);
        let _ = writeln!(
            self.out,
            "{}",
            crate::color::bold(&format!(
                "{path} — {}",
                match why {
                    WholeFile::Binary => "binary, changed on both sides",
                    WholeFile::DeletedByTheirs => "changed here, deleted there",
                    WholeFile::DeletedByMine => "deleted here, changed there",
                }
            ))
        );
        loop {
            let _ = write!(
                self.out,
                "  [m]ine: {mine} ({})  [t]heirs: {theirs} ({})  [q]uit — {} > ",
                labels.mine, labels.theirs, labels.on_quit
            );
            match self.answer()?.as_str() {
                "m" | "mine" => break Ok(Side::Mine),
                "t" | "theirs" => break Ok(Side::Theirs),
                "q" | "quit" => break Err(Stop::Cancelled),
                _ => continue,
            }
        }
    }
}

// -- Editing a region -----------------------------------------------------------
//
// Three steps, separable because front ends differ in the middle one: a
// terminal prompt just runs the editor; a TUI has to give up the screen first.

/// What the user is handed to edit: the collision in conflict markers, with
/// its context either side to orient by.
pub fn edit_buffer(
    collision: &Collision,
    before: &[String],
    after: &[String],
    mine: &str,
    theirs: &str,
) -> String {
    let mut lines: Vec<String> = before.to_vec();
    lines.push(format!("{} mine ({mine})", crate::fuse::MARKER_OURS));
    lines.extend(collision.ours.iter().cloned());
    lines.push(crate::fuse::MARKER_SEP.to_string());
    lines.extend(collision.theirs.iter().cloned());
    lines.push(format!("{} theirs ({theirs})", crate::fuse::MARKER_THEIRS));
    lines.extend(after.iter().cloned());
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Run `editor` on `buffer` in a scratch file and return what was saved.
/// `path` is only a hint: its extension is kept so the editor highlights the
/// text as what it is.
pub fn edit_text(editor: &mut Editor, path: &str, buffer: &str) -> std::io::Result<String> {
    let suffix = Path::new(path)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let file = ScratchFile::create(&suffix, buffer)?;
    editor(&file.0)?;
    std::fs::read_to_string(&file.0)
}

/// Why an edited buffer was not taken as an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditRejected {
    MarkersLeft,
    ContextChanged,
}

impl std::fmt::Display for EditRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EditRejected::MarkersLeft => "The markers are still there.",
            EditRejected::ContextChanged => {
                "The surrounding context lines were changed; only the marked region can be \
                 edited here."
            }
        })
    }
}

/// The region's new lines, out of what the user saved.
///
/// Markers still present mean they have not decided. The context lines were
/// there to orient, not to be merged twice: left alone they are peeled back
/// off, but if they were edited there is no telling what was meant.
pub fn parse_edited(
    edited: &str,
    before: &[String],
    after: &[String],
) -> Result<Vec<String>, EditRejected> {
    if crate::fuse::has_conflict_markers(edited.as_bytes()) {
        return Err(EditRejected::MarkersLeft);
    }
    let lines: Vec<String> = edited.lines().map(str::to_string).collect();
    let (nb, na) = (before.len(), after.len());
    if lines.len() < nb + na || lines[..nb] != *before || lines[lines.len() - na..] != *after {
        return Err(EditRejected::ContextChanged);
    }
    Ok(lines[nb..lines.len() - na].to_vec())
}

/// A uniquely named file in the system temp directory, removed on drop.
struct ScratchFile(std::path::PathBuf);

impl ScratchFile {
    fn create(suffix: &str, content: &str) -> std::io::Result<Self> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "ivaldi-collision-{}-{}{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            suffix
        );
        let path = std::env::temp_dir().join(name);
        // `create_new`: never write through something already at that path.
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let scratch = ScratchFile(path);
        file.write_all(content.as_bytes())?;
        Ok(scratch)
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// An [`Editor`] that runs `$VISUAL`, else `$EDITOR`, else `vi`, on the
/// terminal the user is already at.
pub fn system_editor() -> Editor {
    Box::new(|path| {
        let command = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "vi".into());
        // `EDITOR="code --wait"` is common; split the way a shell would for
        // the simple case and let the rest be arguments.
        let mut parts = command.split_whitespace();
        let program = parts.next().unwrap_or("vi");
        let status = std::process::Command::new(program)
            .args(parts)
            .arg(path)
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "{command} exited with {status}"
            )))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    const LABELS: Labels<'static> = Labels {
        mine: "main",
        theirs: "feature",
        on_quit: "changes nothing",
    };

    fn doc(edits: &[(usize, &str)]) -> String {
        (1..=30)
            .map(|n| match edits.iter().find(|(line, _)| *line == n) {
                Some((_, text)) => format!("{text}\n"),
                None => format!("line {n}\n"),
            })
            .collect()
    }

    fn no_editor() -> Editor {
        Box::new(|_| panic!("editor must not run"))
    }

    fn prompt(answers: &str, editor: Editor) -> PromptResolver<&[u8], Vec<u8>> {
        PromptResolver::new(answers.as_bytes(), Vec::new(), editor)
    }

    fn two_collisions() -> Merge3 {
        Merge3::new(
            &doc(&[]),
            &doc(&[(3, "MINE A"), (25, "MINE B")]),
            &doc(&[(3, "THEIRS A"), (25, "THEIRS B")]),
        )
    }

    #[test]
    fn prefer_answers_every_region_the_same_way() {
        let merge = two_collisions();
        for (prefer, a, b) in [
            (Prefer::Mine, "MINE A", "MINE B"),
            (Prefer::Theirs, "THEIRS A", "THEIRS B"),
            (Prefer::Both, "MINE A\nTHEIRS A", "MINE B\nTHEIRS B"),
        ] {
            let r = resolve_regions(&merge, "f", LABELS, &mut PreferResolver(prefer)).unwrap();
            assert_eq!(merge.render(&r, "", ""), doc(&[(3, a), (25, b)]));
        }
    }

    #[test]
    fn prefer_both_refuses_a_file_that_cannot_hold_both() {
        let stop = PreferResolver(Prefer::Both)
            .whole_file("logo.png", WholeFile::Binary, LABELS)
            .unwrap_err();
        assert!(matches!(stop, Stop::Unresolvable(m) if m.contains("logo.png")));
    }

    #[test]
    fn prompt_takes_a_different_answer_per_collision() {
        let merge = two_collisions();
        let mut resolver = prompt("t\nnonsense\nb\n", no_editor());
        let r = resolve_regions(&merge, "src/f.rs", LABELS, &mut resolver).unwrap();
        assert_eq!(
            merge.render(&r, "", ""),
            doc(&[(3, "THEIRS A"), (25, "MINE B\nTHEIRS B")])
        );

        let shown = String::from_utf8(resolver.out).unwrap();
        assert!(
            shown.contains("src/f.rs — collision 1 of 2, around line 3"),
            "{shown}"
        );
        assert!(
            shown.contains("collision 2 of 2, around line 25"),
            "{shown}"
        );
        assert!(shown.contains("mine (main)") && shown.contains("theirs (feature)"));
        assert!(
            shown.contains("line 2") && shown.contains("line 4"),
            "context: {shown}"
        );
    }

    #[test]
    fn quitting_or_running_out_of_input_cancels_rather_than_guessing() {
        let merge = two_collisions();
        for answers in ["q\n", "m\nq\n", "m\n", ""] {
            let stop = resolve_regions(&merge, "f", LABELS, &mut prompt(answers, no_editor()));
            assert_eq!(stop.unwrap_err(), Stop::Cancelled, "answers: {answers:?}");
        }
    }

    #[test]
    fn edit_takes_the_region_the_user_wrote_and_nothing_else() {
        let merge = Merge3::new(&doc(&[]), &doc(&[(3, "MINE")]), &doc(&[(3, "THEIRS")]));
        let editor: Editor = Box::new(|path| {
            let text = std::fs::read_to_string(path).unwrap();
            assert!(text.contains("<<<<<<< mine (main)") && text.contains("THEIRS"));
            // What a person does: delete the markers, write the combination.
            let combined: String = text
                .lines()
                .filter(|l| !l.starts_with("<<<<<<<") && !l.starts_with(">>>>>>>"))
                .filter(|l| *l != "=======" && *l != "THEIRS")
                .map(|l| format!("{}\n", l.replace("MINE", "MINE && THEIRS")))
                .collect();
            std::fs::write(path, combined)
        });
        let r = resolve_regions(&merge, "f.rs", LABELS, &mut prompt("e\n", editor)).unwrap();
        assert_eq!(r, [Some(Resolution::Custom(vec!["MINE && THEIRS".into()]))]);
        assert_eq!(merge.render(&r, "", ""), doc(&[(3, "MINE && THEIRS")]));
    }

    #[test]
    fn edit_left_with_markers_or_mangled_context_asks_again() {
        let merge = Merge3::new(&doc(&[]), &doc(&[(3, "MINE")]), &doc(&[(3, "THEIRS")]));

        let mut calls = VecDeque::from(["untouched", "mangled"]);
        let editor: Editor = Box::new(move |path| match calls.pop_front().unwrap() {
            "untouched" => Ok(()),
            _ => std::fs::write(path, "just this\n"),
        });
        let mut resolver = prompt("e\ne\nm\n", editor);
        let r = resolve_regions(&merge, "f", LABELS, &mut resolver).unwrap();
        assert_eq!(r, [Some(Resolution::Ours)]);

        let shown = String::from_utf8(resolver.out).unwrap();
        assert!(shown.contains("markers are still there"), "{shown}");
        assert!(shown.contains("context lines were changed"), "{shown}");
    }

    #[test]
    fn whole_file_prompt_explains_what_each_side_means() {
        let mut resolver = prompt("x\nt\n", no_editor());
        let side = resolver
            .whole_file("a.txt", WholeFile::DeletedByTheirs, LABELS)
            .unwrap();
        assert_eq!(side, Side::Theirs);
        let shown = String::from_utf8(resolver.out).unwrap();
        assert!(shown.contains("delete it, as they did"), "{shown}");
        assert!(shown.contains("[q]uit — changes nothing"), "{shown}");
    }

    #[test]
    fn questions_list_every_collision_and_apply_the_answers() {
        let dir = tempfile::tempdir().unwrap();
        let cas = crate::cas::FileCas::new(dir.path().join("objects")).unwrap();
        let store = FsStore::new(&cas);
        let put = |bytes: &[u8]| Some(store.put_blob(bytes).unwrap().0);
        let conflicts = [
            Conflict {
                path: "a.rs".into(),
                base: put(doc(&[]).as_bytes()),
                ours: put(doc(&[(3, "MINE A"), (25, "MINE B"), (14, "MINE ONLY")]).as_bytes()),
                theirs: put(doc(&[(3, "THEIRS A"), (25, "THEIRS B")]).as_bytes()),
            },
            Conflict {
                path: "gone.txt".into(),
                base: put(b"base\n"),
                ours: put(b"changed\n"),
                theirs: None,
            },
            Conflict {
                path: "logo.png".into(),
                base: put(b"\x00b"),
                ours: put(b"\x00mine"),
                theirs: put(b"\x00theirs"),
            },
        ];
        let mut q = Questions::new(&store, &conflicts).unwrap();
        assert_eq!(q.len(), 4, "two regions, a deletion, a binary");

        match q.get(1).unwrap() {
            Question::Region {
                path,
                index,
                total,
                collision,
                before,
                ..
            } => {
                assert_eq!((path, index, total), ("a.rs", 2, 2));
                assert_eq!(collision.ours, ["MINE B"]);
                assert_eq!(before.last().map(String::as_str), Some("line 24"));
            }
            Question::WholeFile { .. } => panic!("expected a region"),
        }
        assert!(matches!(
            q.get(2),
            Some(Question::WholeFile {
                path: "gone.txt",
                why: WholeFile::DeletedByTheirs
            })
        ));
        assert!(q.get(4).is_none());

        // Nothing is applied while anything is unanswered.
        let mut merged = std::collections::BTreeMap::new();
        assert!(q.apply(&store, &mut merged).is_err());
        assert!(merged.is_empty());

        // The wrong kind of answer is not recorded.
        assert!(!q.answer(0, Answer::WholeFile(Side::Mine)));
        assert!(!q.answer(2, Answer::Region(Resolution::Both)));
        assert_eq!(q.next_unanswered(0), Some(0));

        assert!(q.answer(1, Answer::Region(Resolution::Both)));
        assert_eq!(q.next_unanswered(1), Some(2), "skips what is answered");
        assert!(q.answer(0, Answer::Region(Resolution::Theirs)));
        assert!(q.answer(2, Answer::WholeFile(Side::Theirs)));
        assert!(q.answer(3, Answer::WholeFile(Side::Mine)));
        assert_eq!(q.next_unanswered(0), None);

        merged.insert("gone.txt".into(), B3Hash::digest(b"stale"));
        q.apply(&store, &mut merged).unwrap();
        let load = |path: &str| store.load_blob(merged[path]).unwrap().1;
        assert_eq!(
            load("a.rs"),
            doc(&[(3, "THEIRS A"), (14, "MINE ONLY"), (25, "MINE B\nTHEIRS B")]).as_bytes()
        );
        assert!(!merged.contains_key("gone.txt"), "theirs deleted it");
        assert_eq!(load("logo.png"), b"\x00mine");
    }

    #[test]
    fn answers_given_up_front_are_replayed_only_to_the_same_question() {
        let asked = two_collisions();
        let mut questions = Questions::from_merges(vec![
            ("f".into(), asked.clone()),
            ("clean".into(), Merge3::new("a\n", "b\n", "a\n")),
        ]);
        assert_eq!(questions.len(), 2, "a merge with no collision asks nothing");
        questions.answer(0, Answer::Region(Resolution::Theirs));
        questions.answer(1, Answer::Region(Resolution::Custom(vec!["EDITED".into()])));
        let mut replay = questions.into_resolver();

        let r = resolve_regions(&asked, "f", LABELS, &mut replay).unwrap();
        assert_eq!(
            asked.render(&r, "", ""),
            doc(&[(3, "THEIRS A"), (25, "EDITED")])
        );

        // Same path, but the file moved on since the question was put.
        let changed = Merge3::new(
            &doc(&[]),
            &doc(&[(3, "MINE, REWRITTEN SINCE"), (25, "MINE B")]),
            &doc(&[(3, "THEIRS A"), (25, "THEIRS B")]),
        );
        assert!(matches!(
            resolve_regions(&changed, "f", LABELS, &mut replay),
            Err(Stop::Unresolvable(_))
        ));
        assert!(matches!(
            resolve_regions(&asked, "other", LABELS, &mut replay),
            Err(Stop::Unresolvable(_))
        ));
    }

    #[test]
    fn edit_buffer_round_trips_through_parse() {
        let merge = Merge3::new(&doc(&[]), &doc(&[(3, "MINE")]), &doc(&[(3, "THEIRS")]));
        let (collision, before, after) = collisions_in_context(&merge).next().unwrap();
        let buffer = edit_buffer(collision, before, after, "main", "feature");
        assert!(buffer.starts_with("line 1\nline 2\n<<<<<<< mine (main)\nMINE\n=======\n"));
        assert!(buffer.ends_with(">>>>>>> theirs (feature)\nline 4\nline 5\nline 6\n"));

        assert_eq!(
            parse_edited(&buffer, before, after),
            Err(EditRejected::MarkersLeft)
        );
        assert_eq!(
            parse_edited(
                "line 1\nline 2\nA\nB\nline 4\nline 5\nline 6\n",
                before,
                after
            ),
            Ok(vec!["A".to_string(), "B".to_string()])
        );
        // Deleting the region outright is an answer.
        assert_eq!(
            parse_edited("line 1\nline 2\nline 4\nline 5\nline 6\n", before, after),
            Ok(Vec::new())
        );
        assert_eq!(
            parse_edited("line 1\nA\nline 4\nline 5\nline 6\n", before, after),
            Err(EditRejected::ContextChanged)
        );
    }

    #[test]
    fn resolve_conflict_covers_text_binary_and_deletion() {
        let dir = tempfile::tempdir().unwrap();
        let cas = crate::cas::FileCas::new(dir.path().join("objects")).unwrap();
        let store = FsStore::new(&cas);
        let put = |bytes: &[u8]| Some(store.put_blob(bytes).unwrap().0);
        let conflict = |base, ours, theirs| Conflict {
            path: "f".into(),
            base,
            ours,
            theirs,
        };
        let load = |hash: Option<B3Hash>| store.load_blob(hash.unwrap()).unwrap().1;

        let text = conflict(
            put(doc(&[]).as_bytes()),
            put(doc(&[(3, "MINE"), (20, "MINE ONLY")]).as_bytes()),
            put(doc(&[(3, "THEIRS")]).as_bytes()),
        );
        let merged =
            resolve_conflict(&store, &text, LABELS, &mut PreferResolver(Prefer::Theirs)).unwrap();
        // Only the collision goes to theirs; mine's other edit is kept.
        assert_eq!(
            load(merged),
            doc(&[(3, "THEIRS"), (20, "MINE ONLY")]).as_bytes()
        );

        let binary = conflict(put(b"\x00b"), put(b"\x00mine"), put(b"\x00theirs"));
        let kept =
            resolve_conflict(&store, &binary, LABELS, &mut PreferResolver(Prefer::Mine)).unwrap();
        assert_eq!(load(kept), b"\x00mine");

        let deleted_there = conflict(put(b"base\n"), put(b"changed\n"), None);
        for (prefer, expect_kept) in [(Prefer::Mine, true), (Prefer::Theirs, false)] {
            let result =
                resolve_conflict(&store, &deleted_there, LABELS, &mut PreferResolver(prefer))
                    .unwrap();
            assert_eq!(result.is_some(), expect_kept);
        }
    }
}
