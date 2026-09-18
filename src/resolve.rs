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
    const CONTEXT: usize = 3;
    let total = merge.collisions();
    let mut resolutions = Vec::with_capacity(total);

    for (i, chunk) in merge.chunks.iter().enumerate() {
        let Chunk::Collision(collision) = chunk else {
            continue;
        };
        let before = match i.checked_sub(1).map(|j| &merge.chunks[j]) {
            Some(Chunk::Clean(lines)) => &lines[lines.len().saturating_sub(CONTEXT)..],
            _ => &[],
        };
        let after = match merge.chunks.get(i + 1) {
            Some(Chunk::Clean(lines)) => &lines[..lines.len().min(CONTEXT)],
            _ => &[],
        };
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
        let failed = |e: std::io::Error| Stop::Unresolvable(format!("could not run editor: {e}"));
        let c = region.collision;
        let mut text = String::new();
        let mut push = |line: &str| {
            text.push_str(line);
            text.push('\n');
        };
        region.before.iter().for_each(|l| push(l));
        push(&format!(
            "{} mine ({})",
            crate::fuse::MARKER_OURS,
            region.labels.mine
        ));
        c.ours.iter().for_each(|l| push(l));
        push(crate::fuse::MARKER_SEP);
        c.theirs.iter().for_each(|l| push(l));
        push(&format!(
            "{} theirs ({})",
            crate::fuse::MARKER_THEIRS,
            region.labels.theirs
        ));
        region.after.iter().for_each(|l| push(l));

        // Keep the extension so the editor highlights it as what it is.
        let suffix = Path::new(region.path)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let file = ScratchFile::create(&suffix, &text).map_err(failed)?;
        (self.editor)(&file.0).map_err(failed)?;
        let edited = std::fs::read_to_string(&file.0).map_err(failed)?;

        if crate::fuse::has_conflict_markers(edited.as_bytes()) {
            let _ = writeln!(
                self.out,
                "  The markers are still there — not taken as an answer."
            );
            return Ok(None);
        }
        // The context lines were there to orient, not to be merged twice. If
        // the user left them alone, peel them back off; if they edited them,
        // there is no telling what they meant, so ask again.
        let lines: Vec<String> = edited.lines().map(str::to_string).collect();
        let (nb, na) = (region.before.len(), region.after.len());
        if lines.len() < nb + na
            || lines[..nb] != *region.before
            || lines[lines.len() - na..] != *region.after
        {
            let _ = writeln!(
                self.out,
                "  The surrounding context lines were changed; only the marked region can be \
                 edited here. Not taken as an answer."
            );
            return Ok(None);
        }
        Ok(Some(lines[nb..lines.len() - na].to_vec()))
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
