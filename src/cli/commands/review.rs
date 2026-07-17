//! Code review commands.

use super::*;

pub(super) fn parse_repo_arg(arg: &str) -> Result<crate::portal::RepoSpec, String> {
    crate::portal::parse_repo_spec(arg).map_err(|e| {
        format!(
            "invalid repository: '{}' ({})\n  accepted formats:\n    owner/repo\n    https://github.com/owner/repo\n    git@github.com:owner/repo.git\n    github:owner/repo",
            arg, e
        )
    })
}

pub(super) fn cmd_review(args: ReviewArgs, quiet: bool) -> Result<(), String> {
    use crate::review::{self, ReviewFilter, ReviewStatus};

    match args.command {
        ReviewCommands::Create(create_args) => {
            let repo = open_repo()?;
            let review = review::create_review(
                &repo,
                &create_args.title,
                &create_args.description,
                &create_args.source,
                &create_args.target,
                &create_args.strategy,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!(
                    "Created review #{}: {} ({} -> {})",
                    review.id, review.title, review.source_timeline, review.target_timeline
                );
            }
            Ok(())
        }
        ReviewCommands::List(list_args) => {
            let repo = open_repo()?;
            let filter = if list_args.all {
                ReviewFilter::default()
            } else if let Some(ref status_str) = list_args.status {
                ReviewFilter {
                    status: Some(
                        status_str
                            .parse::<ReviewStatus>()
                            .map_err(|_| format!("unknown status: {}", status_str))?,
                    ),
                }
            } else {
                // Default: show non-merged, non-closed
                ReviewFilter::default()
            };

            let reviews = review::list_reviews(&repo, &filter).map_err(|e| e.to_string())?;

            // When not --all and no explicit status, filter out merged/closed
            let reviews: Vec<_> = if !list_args.all && list_args.status.is_none() {
                reviews
                    .into_iter()
                    .filter(|r| {
                        r.status != ReviewStatus::Merged && r.status != ReviewStatus::Closed
                    })
                    .collect()
            } else {
                reviews
            };

            if reviews.is_empty() {
                println!("No reviews found.");
            } else {
                for r in &reviews {
                    println!(
                        "[{}] #{} {} ({} -> {}) by {}",
                        r.status.symbol(),
                        r.id,
                        r.title,
                        r.source_timeline,
                        r.target_timeline,
                        r.author,
                    );
                }
                println!("\n{} review(s)", reviews.len());
            }
            Ok(())
        }
        ReviewCommands::Show(show_args) => {
            let repo = open_repo()?;
            let review = repo
                .load_review(show_args.id)
                .map_err(|e| e.to_string())?
                .ok_or(format!("review #{} not found", show_args.id))?;

            println!("Review #{}: {}", review.id, review.title);
            println!("Status:  {}", review.status);
            println!("Author:  {}", review.author);
            println!(
                "Source:  {} ({})",
                review.source_timeline, review.source_head_seal
            );
            println!(
                "Target:  {} ({})",
                review.target_timeline, review.target_head_seal
            );
            println!("Strategy: {}", review.fuse_strategy);
            if let Some(ref seal) = review.merge_seal {
                println!("Merged:  {}", seal);
            }
            if !review.description.is_empty() {
                println!("\n{}", review.description);
            }

            if !review.comments.is_empty() {
                println!("\n--- Comments ({}) ---", review.comments.len());
                for c in &review.comments {
                    let location = if let Some(line) = c.line {
                        format!("{}:{}", c.path, line)
                    } else {
                        c.path.clone()
                    };
                    let reply = if let Some(rid) = c.reply_to {
                        format!(" (reply to #{})", rid)
                    } else {
                        String::new()
                    };
                    println!("  [{}] {} @ {}{}", c.id, c.author, location, reply);
                    println!("    {}", c.body);
                }
            }

            if !review.verdicts.is_empty() {
                println!("\n--- Verdicts ({}) ---", review.verdicts.len());
                for v in &review.verdicts {
                    println!(
                        "  {} - {} {}",
                        v.status,
                        v.author,
                        if v.body.is_empty() { "" } else { &v.body }
                    );
                }
            }
            Ok(())
        }
        ReviewCommands::Diff(diff_args) => {
            let repo = open_repo()?;
            let changes = review::review_diff(&repo, diff_args.id).map_err(|e| e.to_string())?;

            if changes.is_empty() {
                println!("No changes between source and target.");
                return Ok(());
            }

            if diff_args.stat {
                let mut added = 0usize;
                let mut deleted = 0usize;
                let mut modified = 0usize;
                for c in &changes {
                    match c.kind {
                        crate::fsmerkle::ChangeKind::Added => added += 1,
                        crate::fsmerkle::ChangeKind::Deleted => deleted += 1,
                        crate::fsmerkle::ChangeKind::Modified
                        | crate::fsmerkle::ChangeKind::TypeChange => modified += 1,
                    }
                }
                println!(
                    "{} file(s) changed: {} added, {} deleted, {} modified",
                    changes.len(),
                    added,
                    deleted,
                    modified
                );
            } else {
                for c in &changes {
                    let marker = match c.kind {
                        crate::fsmerkle::ChangeKind::Added => "++",
                        crate::fsmerkle::ChangeKind::Deleted => "--",
                        crate::fsmerkle::ChangeKind::Modified
                        | crate::fsmerkle::ChangeKind::TypeChange => "~~",
                    };
                    println!("{} {}", marker, c.path);
                }
            }
            Ok(())
        }
        ReviewCommands::Comment(comment_args) => {
            let repo = open_repo()?;
            review::add_comment(
                &repo,
                comment_args.id,
                &comment_args.file,
                comment_args.line,
                &comment_args.body,
                comment_args.reply_to,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!("Comment added to review #{}", comment_args.id);
            }
            Ok(())
        }
        ReviewCommands::Approve(approve_args) => {
            let repo = open_repo()?;
            review::submit_verdict(
                &repo,
                approve_args.id,
                ReviewStatus::Approved,
                &approve_args.body,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!("Review #{} approved", approve_args.id);
            }
            Ok(())
        }
        ReviewCommands::RequestChanges(rc_args) => {
            let repo = open_repo()?;
            review::submit_verdict(
                &repo,
                rc_args.id,
                ReviewStatus::ChangesRequested,
                &rc_args.body,
            )
            .map_err(|e| e.to_string())?;

            if !quiet {
                println!("Changes requested on review #{}", rc_args.id);
            }
            Ok(())
        }
        ReviewCommands::Merge(merge_args) => {
            let mut repo = open_repo()?;

            // Optionally override the strategy stored in the review
            if let Some(ref strategy) = merge_args.strategy {
                let mut review = repo
                    .load_review(merge_args.id)
                    .map_err(|e| e.to_string())?
                    .ok_or(format!("review #{} not found", merge_args.id))?;
                review.fuse_strategy = strategy.clone();
                repo.save_review(&review).map_err(|e| e.to_string())?;
            }

            let review =
                review::merge_review(&mut repo, merge_args.id).map_err(|e| e.to_string())?;

            if !quiet {
                println!(
                    "Review #{} merged! Seal: {}",
                    review.id,
                    review.merge_seal.as_deref().unwrap_or("unknown")
                );
            }
            Ok(())
        }
        ReviewCommands::Close(close_args) => {
            let repo = open_repo()?;
            review::close_review(&repo, close_args.id).map_err(|e| e.to_string())?;

            if !quiet {
                println!("Review #{} closed", close_args.id);
            }
            Ok(())
        }
        ReviewCommands::Reopen(reopen_args) => {
            let repo = open_repo()?;
            review::reopen_review(&repo, reopen_args.id).map_err(|e| e.to_string())?;

            if !quiet {
                println!("Review #{} reopened", reopen_args.id);
            }
            Ok(())
        }
    }
}
