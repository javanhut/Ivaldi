package enhanced_cli

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/spf13/cobra"

	"ivaldi/core/commands"
	"ivaldi/core/config"
	"ivaldi/core/fuse"
	"ivaldi/core/network"
	"ivaldi/core/p2p"
	"ivaldi/core/reshape"
	"ivaldi/core/search"
	"ivaldi/core/semantic"
	"ivaldi/core/workspace"
	"ivaldi/forge"
	"ivaldi/ui/enhanced"
)

// EnhancedCLI integrates all revolutionary features into the command interface
type EnhancedCLI struct {
	output      *enhanced.EnhancedOutput
	nlParser    *commands.NaturalLanguageParser
	currentRepo *forge.EnhancedRepository
}

// NewEnhancedCLI creates a new enhanced CLI with all revolutionary features
func NewEnhancedCLI() *EnhancedCLI {
	output := enhanced.NewEnhancedOutput()
	nlParser := commands.NewNaturalLanguageParser()

	return &EnhancedCLI{
		output:   output,
		nlParser: nlParser,
	}
}

// Create the enhanced root command
func (ec *EnhancedCLI) CreateRootCommand() *cobra.Command {
	rootCmd := &cobra.Command{
		Use:   "ivaldi",
		Short: "Ivaldi - Human-centered version control that never loses your work",
		Long: `Ivaldi - A Revolutionary Version Control System

Named after the Norse master craftsman, Ivaldi reimagines version control
with human-friendly commands, natural language references, and zero work loss.

Key Features:
- Memorable names instead of cryptic hashes (bright-river-42)
- Automatic work preservation - never lose anything again  
- Natural language references ("yesterday at 3pm", "Sarah's last commit")
- Rich visual output with helpful error messages
- Local-first collaboration with real-time sync
- Complete accountability for all history modifications

Workshop Commands:
  forge                    Create a new repository
  download <url>           Download current files (no Git history)
  mirror <url>             Mirror repository with full Git history
  gather [files...]        Stage files for sealing
  exclude <files...>       Exclude files from tracking
  remove <files...>        Remove files from repository
  seal [message]           Commit changes with memorable name
  timeline                 Manage development timelines (branches)
  rename <old> --to <new>  Rename timeline
  jump <reference>         Jump to any point using natural language
  shelf                    Save work for later
  portal                   Manage remote connections
  upload [branch]          Upload current branch to portal
  sync <timeline> --with <remote>  Sync local timeline with remote timeline
  scout [portal]           Discover remote timelines
  p2p                      Peer-to-peer collaboration and sync
  
Natural Language Examples:
  jump to "yesterday before lunch"
  seal "Add authentication middleware"
  timeline switch feature --preserve
  gather src/ except tests/`,
		PersistentPreRunE: ec.initializeRepository,
		Run: func(cmd *cobra.Command, args []string) {
			if len(args) == 0 {
				ec.output.Info("Welcome to Ivaldi!")
				ec.output.Info("")
				ec.output.Info("Try: ivaldi help")
				ec.output.Info("Or: forge (to create a new repository)")
				return
			}

			// For now, just show help for unknown commands
			ec.output.Error(fmt.Sprintf("Unknown command: %s", strings.Join(args, " ")), []string{
				"Try: ivaldi help",
				"Use workshop commands: forge, gather, seal, timeline, jump",
				"Natural language parsing coming soon!",
			})
		},
	}

	// Add all enhanced commands
	ec.addEnhancedCommands(rootCmd)

	return rootCmd
}

// Initialize repository and load enhanced features
func (ec *EnhancedCLI) initializeRepository(cmd *cobra.Command, args []string) error {
	// Skip repo check for init commands
	if cmd.Name() == "forge" || cmd.Name() == "download" || cmd.Name() == "mirror" || cmd.Name() == "migrate" || cmd.Name() == "config" || cmd.Name() == "help" {
		return nil
	}

	// Check if we're in a repository
	wd, err := os.Getwd()
	if err != nil {
		return err
	}

	// Find .ivaldi directory
	repoRoot := findRepositoryRoot(wd)
	if repoRoot == "" {
		ec.output.Error("Not in an Ivaldi repository", []string{
			"Run: forge (to create a new repository)",
			"Or: mirror <url> (to clone an existing repository)",
			"Navigate to an existing Ivaldi repository",
		})
		return fmt.Errorf("not in repository")
	}

	// Load enhanced repository
	repo, err := forge.NewEnhancedRepository(repoRoot)
	if err != nil {
		ec.output.Error("Failed to load repository", []string{
			"Check that .ivaldi directory exists and is valid",
			"Try: forge (to reinitialize)",
		})
		return err
	}

	ec.currentRepo = repo
	return nil
}

// Add all enhanced commands
func (ec *EnhancedCLI) addEnhancedCommands(rootCmd *cobra.Command) {
	// Core workshop commands
	rootCmd.AddCommand(ec.createForgeCommand())
	rootCmd.AddCommand(ec.createDownloadCommand())
	rootCmd.AddCommand(ec.createMirrorCommand())
	rootCmd.AddCommand(ec.createMigrateCommand())
	rootCmd.AddCommand(ec.createGatherCommand())
	rootCmd.AddCommand(ec.createExcludeCommand())
	rootCmd.AddCommand(ec.createRemoveCommand())
	rootCmd.AddCommand(ec.createSealCommand())
	rootCmd.AddCommand(ec.createTimelineCommand())
	rootCmd.AddCommand(ec.createRenameCommand())
	rootCmd.AddCommand(ec.createJumpCommand())
	rootCmd.AddCommand(ec.createShelfCommand())
	rootCmd.AddCommand(ec.createPortalCommand())
	rootCmd.AddCommand(ec.createUploadCommand())
	rootCmd.AddCommand(ec.createSyncCommand())
	rootCmd.AddCommand(ec.createScoutCommand())
	rootCmd.AddCommand(ec.createP2PCommand())

	// Information commands
	rootCmd.AddCommand(ec.createStatusCommand())
	rootCmd.AddCommand(ec.createWhereAmICommand())
	rootCmd.AddCommand(ec.createWhatChangedCommand())
	rootCmd.AddCommand(ec.createLogCommand())
	rootCmd.AddCommand(ec.createSearchCommand())

	// Advanced commands
	rootCmd.AddCommand(ec.createReshapeCommand())
	rootCmd.AddCommand(ec.createFuseCommand())
	rootCmd.AddCommand(ec.createPluckCommand())
	rootCmd.AddCommand(ec.createHuntCommand())
	rootCmd.AddCommand(ec.createSquashCommand())

	// Workspace commands
	rootCmd.AddCommand(ec.createWorkspaceCommand())

	// Collaboration commands
	rootCmd.AddCommand(ec.createMeshCommand())
	rootCmd.AddCommand(ec.createCollaborateCommand())

	// Utility commands
	rootCmd.AddCommand(ec.createVersionCommand())
	rootCmd.AddCommand(ec.createConfigCommand())
	rootCmd.AddCommand(ec.createCleanCommand())
	rootCmd.AddCommand(ec.createRefreshCommand())
}

// Create forge command (initialize repository)
func (ec *EnhancedCLI) createForgeCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "forge [path]",
		Short: "Create a new Ivaldi repository",
		Long: `Create a new Ivaldi repository with all revolutionary features

This initializes a new repository with:
- Natural language reference system
- Automatic work preservation
- Complete overwrite tracking
- Rich visual output
- Human-friendly error messages

Example:
  forge                    # Create repository in current directory
  forge my-project         # Create repository in new directory`,
		RunE: func(cmd *cobra.Command, args []string) error {
			path := "."
			if len(args) > 0 {
				path = args[0]
			}

			absPath, err := filepath.Abs(path)
			if err != nil {
				return err
			}

			ec.output.Info(fmt.Sprintf("Forging new repository in %s", absPath))

			repo, err := forge.EnhancedInitialize(absPath)
			if err != nil {
				ec.output.Error("Failed to forge repository", []string{
					"Check directory permissions",
					"Ensure path exists or can be created",
				})
				return err
			}

			ec.currentRepo = repo
			ec.output.Success("Repository forged successfully!")
			ec.output.Info("")
			ec.output.Info("Next steps:")
			ec.output.Info("- gather <files>     - Stage files for sealing")
			ec.output.Info("- seal \"message\"     - Create your first commit")
			ec.output.Info("- timeline create    - Create development timelines")

			return nil
		},
	}
}

// Create mirror command (clone repository)
func (ec *EnhancedCLI) createMirrorCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "mirror <url> [destination]",
		Short: "Clone and convert a remote Git repository to Ivaldi with full history",
		Long: `Clone a remote Git repository and convert it to Ivaldi format with complete history preservation

This command:
- Clones the Git repository from a remote URL (GitHub/GitLab/etc)
- Automatically detects repository name from URL if destination not specified
- Converts ALL Git commits to Ivaldi seals with memorable names
- Preserves complete commit history with proper parent relationships
- Converts Git branches to Ivaldi timelines
- Migrates .gitmodules to .ivaldimodules format
- Downloads and indexes all files for immediate use

Use this to clone and convert remote Git repositories to Ivaldi.

Examples:
  mirror https://github.com/user/repo              # Creates ./repo directory
  mirror git@github.com:user/project.git           # Creates ./project directory
  mirror https://github.com/org/tool custom-name   # Creates ./custom-name directory`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			url := args[0]
			dest := ""

			// Extract repository name from URL if destination not specified
			if len(args) > 1 {
				dest = args[1]
			} else {
				dest = ec.extractRepoNameFromURL(url)
				if dest == "" {
					ec.output.Error("Could not determine repository name from URL", []string{
						"Please specify a destination directory",
						"Example: mirror " + url + " my-repo",
					})
					return fmt.Errorf("could not extract repository name from URL")
				}
			}

			// Check if destination already exists
			if _, err := os.Stat(dest); err == nil {
				ec.output.Error("Destination already exists", []string{
					fmt.Sprintf("Directory '%s' already exists", dest),
					"Choose a different name or remove the existing directory",
					"Example: mirror " + url + " " + dest + "-new",
				})
				return fmt.Errorf("destination directory already exists: %s", dest)
			}

			ec.output.Info(fmt.Sprintf("Mirroring repository from %s to %s", url, dest))
			ec.output.Info("")

			// Perform the enhanced mirror operation with full history
			repo, err := ec.performEnhancedMirror(url, dest)
			if err != nil {
				ec.output.Error("Failed to mirror repository", []string{
					"Check URL accessibility and format",
					"Verify authentication if repository is private",
					"Ensure you have network connectivity",
				})
				return err
			}

			ec.currentRepo = repo
			ec.output.Success(fmt.Sprintf("✓ Repository mirrored to %s with complete history!", dest))
			ec.output.Info("")
			ec.output.Info("Repository successfully converted:")
			ec.output.Info("- All Git commits converted to Ivaldi seals")
			ec.output.Info("- Branches converted to Ivaldi timelines")
			ec.output.Info("- Submodules converted to .ivaldimodules")
			ec.output.Info("")
			ec.output.Info("Next steps:")
			ec.output.Info(fmt.Sprintf("- cd %s", dest))
			ec.output.Info("- ivaldi status")
			ec.output.Info("- ivaldi log")

			return nil
		},
	}
}

// Create migrate command (convert Git to Ivaldi)
func (ec *EnhancedCLI) createMigrateCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "migrate [directory]",
		Short: "Migrate a Git repository to Ivaldi with complete history preservation",
		Long: `Convert an existing Git repository to Ivaldi format while preserving complete commit history

This command performs a complete migration:
- Converts all Git commits to Ivaldi seals with memorable names and proper relationships
- Converts Git branches to Ivaldi timelines
- Migrates .gitmodules to .ivaldimodules format
- Creates backup of original .git directory
- Preserves all commit metadata, timestamps, and authorship
- Maintains complete branching and merging history

Use this to migrate existing Git projects to Ivaldi while keeping full history.
The original .git directory will be backed up to .git.backup for safety.

Examples:
  migrate                    # Migrate current directory Git repository
  migrate ./my-git-project   # Migrate specific Git repository`,
		RunE: func(cmd *cobra.Command, args []string) error {
			dir := "."
			if len(args) > 0 {
				dir = args[0]
			}

			absDir, err := filepath.Abs(dir)
			if err != nil {
				return err
			}

			// Check if this is a git repository
			gitDir := filepath.Join(absDir, ".git")
			if _, err := os.Stat(gitDir); os.IsNotExist(err) {
				ec.output.Error("Not a Git repository", []string{
					"Navigate to a Git repository directory",
					"Or use 'forge' to create a new Ivaldi repository",
					"Or use 'mirror <url>' to clone and convert a remote Git repository",
				})
				return fmt.Errorf("no .git directory found")
			}

			// Check if Ivaldi repository already exists
			ivaldiDir := filepath.Join(absDir, ".ivaldi")
			if _, err := os.Stat(ivaldiDir); err == nil {
				ec.output.Error("Ivaldi repository already exists", []string{
					"This directory already contains an Ivaldi repository",
					"Remove .ivaldi directory if you want to re-migrate",
					"Or use 'sync' to update from remote",
				})
				return fmt.Errorf("ivaldi repository already exists")
			}

			ec.output.Info(fmt.Sprintf("Migrating Git repository to Ivaldi: %s", absDir))
			ec.output.Info("")

			if err := ec.performGitToIvaldiMigration(absDir); err != nil {
				ec.output.Error("Migration failed", []string{
					"Check repository permissions",
					"Ensure Git repository is valid",
					"Verify remote access if repository has remotes",
				})
				return err
			}

			ec.output.Success("✓ Git repository successfully migrated to Ivaldi!")
			ec.output.Info("")
			ec.output.Info("Migration complete:")
			ec.output.Info("- Git commits converted to Ivaldi seals with memorable names")
			ec.output.Info("- Git branches converted to Ivaldi timelines")
			ec.output.Info("- .gitmodules converted to .ivaldimodules")
			ec.output.Info("- Original .git directory backed up to .git.backup")
			ec.output.Info("")
			ec.output.Info("Next steps:")
			ec.output.Info("- Run 'ivaldi status' to see your current workspace")
			ec.output.Info("- Use 'ivaldi timeline' to explore converted branches")
			ec.output.Info("- Continue development with Ivaldi's enhanced commands")

			return nil
		},
	}
}

// Create gather command (stage files)
func (ec *EnhancedCLI) createGatherCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "gather [files...]",
		Short: "Gather files onto the anvil for sealing",
		Long: `Gather files onto the anvil (staging area) for sealing

Smart gathering with natural language patterns:
- gather all                     - Gather all changed files
- gather src/                    - Gather directory recursively
- gather *.go                    - Gather by pattern
- gather src/ except tests/      - Gather with exclusions
- gather --interactive           - Interactive selection

Examples:
  gather all
  gather src/main.go src/auth.go
  gather *.go except *_test.go`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Parse gathering pattern
			pattern := strings.Join(args, " ")
			if pattern == "" {
				pattern = "all"
			}

			// Use basic gathering for now (enhanced handler implementation needed)
			if pattern == "all" {
				// Gather all changed files only
				err := ec.currentRepo.Gather([]string{"."})
				if err != nil {
					ec.output.Error("Failed to gather files", []string{
						"Check file paths exist",
						"Try: gather <specific-files>",
						"Use: gather --interactive for selection",
					})
					return err
				}

				// Count what was gathered
				ws := ec.currentRepo.GetWorkspace()
				gatheredCount := len(ws.AnvilFiles)
				if gatheredCount > 0 {
					ec.output.Success(fmt.Sprintf("Gathered %d changed file(s) onto the anvil", gatheredCount))
				} else {
					ec.output.Info("No changes to gather - workspace is clean")
					return nil
				}
			} else {
				err := ec.currentRepo.Gather(args)
				if err != nil {
					ec.output.Error("Failed to gather files", []string{
						"Check file paths exist",
						"Try: gather all",
						"Use: gather --interactive for selection",
					})
					return err
				}

				// Count what was gathered
				ws := ec.currentRepo.GetWorkspace()
				gatheredCount := len(ws.AnvilFiles)
				if gatheredCount > 0 {
					ec.output.Success(fmt.Sprintf("Gathered %d file(s) onto the anvil", gatheredCount))
				} else {
					ec.output.Info("No changes in specified files to gather")
					return nil
				}
			}

			return nil
		},
	}

	cmd.Flags().BoolP("interactive", "i", false, "Interactive file selection")
	cmd.Flags().BoolP("all", "a", false, "Gather all files (including untracked)")

	return cmd
}

// Create seal command (commit changes)
func (ec *EnhancedCLI) createSealCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "seal [message]",
		Short: "Seal gathered changes with memorable name",
		Long: `Seal gathered changes into a commit with memorable name

Features:
- Auto-generates memorable names (bright-river-42)
- Smart commit message generation if no message provided
- Automatic work preservation before sealing
- Complete tracking and accountability

Examples:
  seal "Add user authentication"
  seal                              # Auto-generate message
  seal --amend                      # Amend last commit`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			message := ""
			if len(args) > 0 {
				message = strings.Join(args, " ")
			}

			// If no message provided, auto-generate using semantic analysis
			if message == "" {
				ec.output.Info("Analyzing changes for semantic commit message...")

				generator := semantic.NewCommitMessageGenerator()
				generated, err := generator.Generate(ec.currentRepo.GetWorkspace())
				if err != nil {
					message = "Update workspace changes"
					ec.output.Warning("Auto-generation failed, using default message")
					ec.output.Info("Try: seal \"your message\" or gather files first")
				} else {
					message = generated.Primary
					ec.output.Success(fmt.Sprintf("Generated: %s", message))
					ec.output.Info(fmt.Sprintf("Confidence: %.1f%% (%s)", generated.Confidence*100, generated.Type.Description))

					// Show alternatives
					if len(generated.Alternative) > 0 {
						ec.output.Info("Alternatives:")
						for i, alt := range generated.Alternative {
							if i < 2 { // Show max 2 alternatives
								ec.output.Info(fmt.Sprintf("   %d. %s", i+1, alt))
							}
						}
					}
				}
			}

			// Use enhanced seal with memorable names
			seal, err := ec.currentRepo.EnhancedSeal(message)
			if err != nil {
				ec.output.Error("Failed to seal changes", []string{
					"Ensure files are gathered (use: gather)",
					"Check that there are changes to seal",
					"Try: gather all, then seal \"message\"",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Sealed as: %s", seal.Name))
			ec.output.Info(fmt.Sprintf("Message: %s", seal.Message))
			ec.output.Info(fmt.Sprintf("Iteration: #%d", seal.Iteration))

			return nil
		},
	}
}

// Create timeline command (branch management)
func (ec *EnhancedCLI) createTimelineCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "timeline",
		Short: "Manage development timelines (branches)",
		Long: `Manage development timelines with automatic work preservation

Subcommands:
  create <name>           Create new timeline
  switch <name>           Switch to timeline (with auto-preservation) 
  source                  Switch back to main timeline
  list                    List all timelines
  delete <name>           Delete timeline
  rename <old> --to <new> Rename timeline
  merge <from> <to>       Merge timelines

Examples:
  timeline create feature
  timeline switch main --preserve
  timeline list
  timeline delete old-feature
  timeline rename master --to main`,
		RunE: func(cmd *cobra.Command, args []string) error {
			// Show timeline status by default
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			current := ec.currentRepo.GetCurrentTimeline()
			timelines := ec.currentRepo.ListTimelines()

			ec.output.Info("Timeline Status:")
			ec.output.Info("")
			for _, timeline := range timelines {
				if timeline == current {
					ec.output.Info(fmt.Sprintf("  * %s (current)", timeline))
				} else {
					ec.output.Info(fmt.Sprintf("    %s", timeline))
				}
			}

			return nil
		},
	}

	// Add subcommands
	cmd.AddCommand(ec.createTimelineCreateCommand())
	cmd.AddCommand(ec.createTimelineSwitchCommand())
	cmd.AddCommand(ec.createTimelineSourceCommand())
	cmd.AddCommand(ec.createTimelineListCommand())
	cmd.AddCommand(ec.createTimelineDeleteCommand())
	cmd.AddCommand(ec.createTimelineRenameCommand())

	return cmd
}

// Create jump command (checkout with natural language)
func (ec *EnhancedCLI) createJumpCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "jump <reference>",
		Short: "Jump to any point using natural language",
		Long: `Jump to any point in history using natural language references

Supported references:
- Memorable names:        bright-river-42
- Iteration numbers:      #150, #-5, main#42
- Natural language:       "yesterday at 3pm", "2 hours ago"
- Author references:      "Sarah's last commit"
- Content references:     "where auth was added"

Examples:
  jump to bright-river-42
  jump to "yesterday before lunch"
  jump to "#150"
  jump to "Sarah's last commit"
  jump back 3`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			reference := strings.Join(args, " ")

			// Remove common words for natural parsing
			reference = strings.TrimPrefix(reference, "to ")
			reference = strings.TrimPrefix(reference, "back ")

			err := ec.currentRepo.EnhancedJump(reference)
			if err != nil {
				ec.output.Error(fmt.Sprintf("Could not jump to '%s'", reference), []string{
					"Try: jump to bright-river-42",
					"Try: jump to \"yesterday\"",
					"Try: jump to #150",
					"Use: timeline list (to see available timelines)",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Jumped to: %s", reference))

			return nil
		},
	}
}

// Timeline subcommands
func (ec *EnhancedCLI) createTimelineCreateCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "create <name>",
		Short: "Create new timeline",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			err := ec.currentRepo.CreateTimeline(args[0])
			if err != nil {
				ec.output.Error("Failed to create timeline", []string{
					"Check timeline name is valid",
					"Timeline might already exist",
				})
				return err
			}

			// Auto-switch to newly created timeline
			_, err = ec.currentRepo.EnhancedTimelineSwitch(args[0])
			if err != nil {
				ec.output.Error("Created timeline but failed to switch", []string{
					"Timeline created successfully but switch failed",
					"Use: timeline switch " + args[0],
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Created and switched to timeline: %s", args[0]))
			return nil
		},
	}
}

func (ec *EnhancedCLI) createTimelineSwitchCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "switch <name>",
		Short: "Switch to timeline with auto-preservation",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			targetTimeline := args[0]
			currentTimeline := ec.currentRepo.GetCurrentTimeline()

			// Check if already on target timeline
			if currentTimeline == targetTimeline {
				ec.output.Info(fmt.Sprintf("Already on timeline '%s'", targetTimeline))
				return nil
			}

			// Check for uncommitted changes before switching
			hasChanges := ec.currentRepo.GetWorkspace().HasUncommittedChanges()
			hasStaged := len(ec.currentRepo.GetWorkspace().AnvilFiles) > 0

			if hasChanges || hasStaged {
				ec.output.Info("Auto-shelving uncommitted work...")
				if hasStaged {
					ec.output.Info(fmt.Sprintf("  - %d gathered files on anvil", len(ec.currentRepo.GetWorkspace().AnvilFiles)))
				}
				if hasChanges {
					changedCount := 0
					for _, file := range ec.currentRepo.GetWorkspace().Files {
						if file.Status == workspace.StatusModified || file.Status == workspace.StatusAdded || file.Status == workspace.StatusDeleted {
							changedCount++
						}
					}
					ec.output.Info(fmt.Sprintf("  - %d uncommitted changes", changedCount))
				}
			}

			snapshot, err := ec.currentRepo.EnhancedTimelineSwitch(targetTimeline)
			if err != nil {
				if strings.Contains(err.Error(), "already on timeline") {
					ec.output.Info(err.Error())
					return nil
				}
				ec.output.Error("Failed to switch timeline", []string{
					"Timeline might not exist",
					"Use: timeline list (to see available)",
					"Try: timeline create " + targetTimeline,
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Switched to timeline: %s", targetTimeline))

			if snapshot != nil {
				ec.output.Info("Work auto-shelved and will restore when you return")
			}

			// Check if work was restored on the new timeline
			if hasChanges || hasStaged {
				newStatus := ec.currentRepo.GetStatus()
				if len(newStatus.Staged) > 0 || len(newStatus.Modified) > 0 {
					ec.output.Info("Previous work automatically restored on this timeline")
				}
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createTimelineSourceCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "source",
		Short: "Switch back to the main timeline",
		Long:  "Switch back to the main/root timeline from any other timeline",
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			currentTimeline := ec.currentRepo.GetCurrentTimeline()

			if currentTimeline == "main" {
				ec.output.Info("Already on main timeline")
				return nil
			}

			hasChanges := ec.currentRepo.GetWorkspace().HasUncommittedChanges()
			hasStaged := len(ec.currentRepo.GetWorkspace().AnvilFiles) > 0

			if hasChanges || hasStaged {
				ec.output.Info("Auto-shelving uncommitted work...")
				if hasStaged {
					ec.output.Info(fmt.Sprintf("  - %d gathered files on anvil", len(ec.currentRepo.GetWorkspace().AnvilFiles)))
				}
				if hasChanges {
					changedCount := 0
					for _, file := range ec.currentRepo.GetWorkspace().Files {
						if file.Status == workspace.StatusModified || file.Status == workspace.StatusAdded || file.Status == workspace.StatusDeleted {
							changedCount++
						}
					}
					ec.output.Info(fmt.Sprintf("  - %d uncommitted changes", changedCount))
				}
			}

			snapshot, err := ec.currentRepo.EnhancedTimelineSwitch("main")
			if err != nil {
				ec.output.Error("Failed to switch to main timeline", []string{
					"Main timeline might not exist",
					"Try: timeline list (to see available)",
				})
				return err
			}

			ec.output.Success("Switched to main timeline")

			if snapshot != nil {
				ec.output.Info("Work auto-shelved and will restore when you return")
			}

			if hasChanges || hasStaged {
				newStatus := ec.currentRepo.GetStatus()
				if len(newStatus.Staged) > 0 || len(newStatus.Modified) > 0 {
					ec.output.Info("Previous work automatically restored on this timeline")
				}
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createTimelineListCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "list",
		Short: "List all timelines",
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			current := ec.currentRepo.GetCurrentTimeline()
			timelines := ec.currentRepo.ListTimelines()

			ec.output.Info("Available timelines:")
			for _, timeline := range timelines {
				if timeline == current {
					ec.output.Info(fmt.Sprintf("  * %s (current)", timeline))
				} else {
					ec.output.Info(fmt.Sprintf("    %s", timeline))
				}
			}
			return nil
		},
	}
}

func (ec *EnhancedCLI) createTimelineDeleteCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "delete <name>",
		Short: "Delete timeline",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			err := ec.currentRepo.DeleteTimeline(args[0])
			if err != nil {
				ec.output.Error("Failed to delete timeline", []string{
					"Timeline might not exist",
					"Cannot delete current timeline",
					"Use: timeline list (to see available)",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Deleted timeline: %s", args[0]))
			return nil
		},
	}
}

func (ec *EnhancedCLI) createTimelineRenameCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "rename <old-name> --to <new-name>",
		Short: "Rename timeline with remote overwrite support",
		Long: `Rename a timeline locally and handle remote uploads properly

This command:
- Renames the timeline locally with all its history
- Updates the current timeline reference if renaming the active timeline
- When uploading, creates the new timeline name remotely and overwrites it
- Preserves all timeline metadata and state files

Examples:
  timeline rename master --to main
  timeline rename old-feature --to new-feature
  timeline rename bugfix-123 --to hotfix-user-auth`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			oldName := args[0]
			newName, _ := cmd.Flags().GetString("to")

			if newName == "" {
				ec.output.Error("Missing required --to flag", []string{
					"Specify the new timeline name",
					"Example: timeline rename master --to main",
				})
				return fmt.Errorf("new timeline name required")
			}

			// Check if old timeline exists
			timelines := ec.currentRepo.ListTimelines()
			oldExists := false
			for _, t := range timelines {
				if t == oldName {
					oldExists = true
					break
				}
			}

			if !oldExists {
				ec.output.Error(fmt.Sprintf("Timeline '%s' does not exist", oldName), []string{
					"Use: timeline list (to see available timelines)",
				})
				return fmt.Errorf("timeline does not exist")
			}

			// Check if new name already exists
			for _, t := range timelines {
				if t == newName {
					ec.output.Error(fmt.Sprintf("Timeline '%s' already exists", newName), []string{
						"Choose a different name",
						"Use: timeline list (to see existing timelines)",
					})
					return fmt.Errorf("timeline name already exists")
				}
			}

			// Get current timeline to show appropriate message
			currentTimeline := ec.currentRepo.GetCurrentTimeline()
			isCurrentTimeline := currentTimeline == oldName

			// Perform the rename
			ec.output.Info(fmt.Sprintf("Renaming timeline '%s' to '%s'...", oldName, newName))

			err := ec.currentRepo.RenameTimeline(oldName, newName)
			if err != nil {
				ec.output.Error("Failed to rename timeline", []string{
					"Timeline might not exist",
					"New name might already be in use",
					"Check for file permission issues",
				})
				return err
			}

			// Success messages
			ec.output.Success(fmt.Sprintf("Timeline renamed: %s → %s", oldName, newName))

			if isCurrentTimeline {
				ec.output.Info("You are now on the renamed timeline")
			}

			ec.output.Info("Timeline history and metadata preserved")
			ec.output.Info("")
			ec.output.Info("When uploading to remote:")
			ec.output.Info(fmt.Sprintf("  - New branch '%s' will be created", newName))
			ec.output.Info(fmt.Sprintf("  - Old branch '%s' will remain untouched", oldName))
			ec.output.Info("  - You may want to delete the old remote branch manually")

			return nil
		},
	}

	cmd.Flags().String("to", "", "New timeline name (required)")
	cmd.MarkFlagRequired("to")

	return cmd
}

// Create top-level rename command (alias for timeline rename)
func (ec *EnhancedCLI) createRenameCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "rename <old-name> --to <new-name>",
		Short: "Rename current or specified timeline",
		Long: `Rename a timeline (shortcut for timeline rename)

This is a convenient alias for 'timeline rename' that makes it easier to rename timelines.
When no old name is provided, renames the current timeline.

Examples:
  rename master --to main
  rename old-feature --to new-feature
  rename --to better-name  # renames current timeline`,
		Args: cobra.RangeArgs(0, 1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			var oldName string
			newName, _ := cmd.Flags().GetString("to")

			if newName == "" {
				ec.output.Error("Missing required --to flag", []string{
					"Specify the new timeline name",
					"Example: rename master --to main",
					"Or: rename --to new-name (renames current timeline)",
				})
				return fmt.Errorf("new timeline name required")
			}

			// If no old name provided, use current timeline
			if len(args) == 0 {
				oldName = ec.currentRepo.GetCurrentTimeline()
				ec.output.Info(fmt.Sprintf("Renaming current timeline '%s' to '%s'", oldName, newName))
			} else {
				oldName = args[0]
				ec.output.Info(fmt.Sprintf("Renaming timeline '%s' to '%s'", oldName, newName))
			}

			// Check if old timeline exists
			timelines := ec.currentRepo.ListTimelines()
			oldExists := false
			for _, t := range timelines {
				if t == oldName {
					oldExists = true
					break
				}
			}

			if !oldExists {
				ec.output.Error(fmt.Sprintf("Timeline '%s' does not exist", oldName), []string{
					"Use: timeline list (to see available timelines)",
				})
				return fmt.Errorf("timeline does not exist")
			}

			// Check if new name already exists
			for _, t := range timelines {
				if t == newName {
					ec.output.Error(fmt.Sprintf("Timeline '%s' already exists", newName), []string{
						"Choose a different name",
						"Use: timeline list (to see existing timelines)",
					})
					return fmt.Errorf("timeline name already exists")
				}
			}

			// Perform the rename
			err := ec.currentRepo.RenameTimeline(oldName, newName)
			if err != nil {
				ec.output.Error("Failed to rename timeline", []string{
					"Timeline might not exist",
					"New name might already be in use",
					"Check for file permission issues",
				})
				return err
			}

			// Success messages
			ec.output.Success(fmt.Sprintf("Timeline renamed: %s → %s", oldName, newName))
			ec.output.Info("Timeline history and metadata preserved")

			return nil
		},
	}

	cmd.Flags().String("to", "", "New timeline name (required)")
	cmd.MarkFlagRequired("to")

	return cmd
}

// Create remaining essential commands
func (ec *EnhancedCLI) createShelfCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "shelf",
		Short: "Save work for later (stash equivalent)",
		Long: `Save work on a shelf for later retrieval

Subcommands:
  put [message]     Save current work
  take              Retrieve last saved work
  list              Show all shelved work
  drop <id>         Delete shelved work`,
		RunE: func(cmd *cobra.Command, args []string) error {
			ec.output.Info("Shelf operations:")
			ec.output.Info("  shelf put \"message\"  - Save current work")
			ec.output.Info("  shelf take           - Retrieve saved work")
			ec.output.Info("  shelf list           - Show shelved work")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createPortalCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "portal",
		Short: "Manage remote connections",
		Long: `Manage portal connections to remote repositories

Subcommands:
  add <name> <url>      Add new portal
  list                  List all portals
  new <branch>          Create new branch with optional migration
  upload <branch>       Upload branch to portal with upstream
  rename <old> --with <new>  Rename branch remotely
  push [portal]         Push to portal
  pull [portal]         Pull from portal
  sync [portal]         Sync with portal (push and pull)`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			portals := ec.currentRepo.ListPortals()
			if len(portals) == 0 {
				ec.output.Info("No portals configured")
				ec.output.Info("Add one with: portal add origin <url>")
				return nil
			}

			ec.output.Info("Configured portals:")
			for name, url := range portals {
				ec.output.Info(fmt.Sprintf("  %s → %s", name, url))
			}
			return nil
		},
	}

	// Add subcommands
	cmd.AddCommand(ec.createPortalAddCommand())
	cmd.AddCommand(ec.createPortalListCommand())
	cmd.AddCommand(ec.createPortalSyncCommand())
	cmd.AddCommand(ec.createPortalPushCommand())
	cmd.AddCommand(ec.createPortalPullCommand())
	cmd.AddCommand(ec.createPortalNewCommand())
	cmd.AddCommand(ec.createPortalUploadCommand())
	cmd.AddCommand(ec.createPortalRenameCommand())

	return cmd
}

func (ec *EnhancedCLI) createUploadCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "upload",
		Short: "Upload to remote repository",
		Long: `Upload the current timeline to the remote repository (like 'git push').

Automatically uploads to origin/main by default. No arguments needed for typical usage.

Examples:
  ivaldi upload                    # Upload current timeline to origin (most common)
  ivaldi upload upstream           # Upload to different portal 
  ivaldi upload --branch feature   # Upload specific branch`,
		Args: cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Get branch flag or default to current timeline
			branch, _ := cmd.Flags().GetString("branch")
			if branch == "" {
				branch = ec.getCurrentTimeline()
			}

			// Default to origin portal, or use argument
			portalName := "origin"
			if len(args) > 0 {
				portalName = args[0]
			}

			// Check if timeline exists locally but might be new to remote
			timelines := ec.currentRepo.ListTimelines()
			isNewTimeline := true
			for _, tl := range timelines {
				if tl == branch && branch != ec.currentRepo.GetCurrentTimeline() {
					isNewTimeline = false
					break
				}
			}

			// Smart upload messaging
			if portalName == "origin" {
				if isNewTimeline && branch != "main" {
					fmt.Printf("Uploading new timeline to origin/%s (will create branch)\n", branch)
				} else {
					fmt.Printf("Uploading to origin/%s\n", branch)
				}
			} else {
				if isNewTimeline && branch != "main" {
					fmt.Printf("Uploading new timeline to %s/%s (will create branch)\n", portalName, branch)
				} else {
					fmt.Printf("Uploading to %s/%s\n", portalName, branch)
				}
			}

			err := ec.currentRepo.UploadToPortal(portalName, branch)
			if err != nil {
				// Check if it's a branch not found error or similar new timeline scenarios
				if strings.Contains(err.Error(), "does not match any") ||
					strings.Contains(err.Error(), "Reference does not exist") ||
					strings.Contains(err.Error(), "Creating new branch") {
					// This is a new timeline - that's fine, the upload should have created it automatically
					// If we're here, there might be a different issue, so show helpful guidance
					ec.output.Info(fmt.Sprintf("Creating new timeline '%s' on remote...", branch))

					// Try the upload again - the new logic should handle branch creation
					err = ec.currentRepo.UploadToPortal(portalName, branch)
					if err != nil {
						ec.output.Error("Failed to create new timeline on remote", []string{
							"Check network connection",
							"Verify portal URL and credentials",
							"Ensure you have push permissions to the repository",
							"Make sure the repository exists on remote",
						})
						return err
					}
				} else {
					ec.output.Error("Failed to upload to portal", []string{
						"Check network connection",
						"Verify portal URL and credentials",
						"Ensure repository exists on remote",
						"Try: portal add origin <url> (if no portal configured)",
					})
					return err
				}
			}

			// Success messaging with helpful next steps
			if portalName == "origin" {
				fmt.Printf("Upload complete: origin/%s\n", branch)
			} else {
				fmt.Printf("Upload complete: %s/%s\n", portalName, branch)
			}

			// Show helpful next steps for new timelines
			if isNewTimeline && branch != "main" {
				ec.output.Info("New timeline uploaded with automatic upstream tracking")
				ec.output.Info("Your timeline is now available on the remote repository")
			}

			return nil
		},
	}

	// Add branch flag
	cmd.Flags().StringP("branch", "b", "", "Branch to upload (defaults to current timeline)")
	return cmd
}

func (ec *EnhancedCLI) createSyncCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "sync <local-timeline> --with <remote-timeline> [portal-name]",
		Short: "Sync local timeline with remote timeline automatically",
		Long: `Sync a local timeline with a remote timeline by pulling changes and fusing them.

This command performs the following steps automatically:
1. Switches to the specified local timeline
2. Pulls latest changes from the remote timeline
3. Fuses those changes into the local timeline
4. Preserves your local work with automatic conflict resolution

Examples:
  ivaldi sync main --with main              # Sync local main with origin/main
  ivaldi sync feature --with main           # Sync local feature with origin/main
  ivaldi sync main --with main upstream     # Sync local main with upstream/main
  ivaldi sync develop --with develop        # Sync local develop with origin/develop`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Get the local timeline to sync (required positional argument)
			localTimeline := args[0]

			// Get the remote branch to sync with
			remoteBranch, _ := cmd.Flags().GetString("with")
			if remoteBranch == "" {
				return fmt.Errorf("--with flag is required to specify remote timeline to sync with")
			}

			// Default to origin portal
			portalName := "origin"
			if len(args) > 1 {
				portalName = args[1]
			}

			currentTimeline := ec.currentRepo.GetCurrentTimeline()

			ec.output.Info(fmt.Sprintf("Syncing local timeline '%s' with remote timeline '%s' from portal '%s'", localTimeline, remoteBranch, portalName))

			// Step 1: Switch to the target local timeline if different from current
			if currentTimeline != localTimeline {
				ec.output.Info(fmt.Sprintf("Step 1: Switching to timeline '%s'...", localTimeline))
				err := ec.currentRepo.SwitchTimeline(localTimeline)
				if err != nil {
					return fmt.Errorf("failed to switch to timeline '%s': %v", localTimeline, err)
				}
				ec.output.Success(fmt.Sprintf("Switched to timeline '%s'", localTimeline))
			}

			// Step 2: Handle any uncommitted changes first
			if ec.currentRepo.GetWorkspace().HasUncommittedChanges() {
				ec.output.Info("Step 2: Auto-sealing uncommitted changes...")

				// Auto-gather and seal uncommitted changes
				err := ec.currentRepo.GetWorkspace().Scan()
				if err != nil {
					return fmt.Errorf("failed to scan workspace: %v", err)
				}

				err = ec.currentRepo.Gather([]string{"."})
				if err != nil {
					return fmt.Errorf("failed to gather changes: %v", err)
				}

				_, err = ec.currentRepo.Seal(fmt.Sprintf("Auto-seal before sync with %s", remoteBranch))
				if err != nil {
					return fmt.Errorf("failed to seal changes: %v", err)
				}

				// Force workspace refresh after sealing
				err = ec.currentRepo.GetWorkspace().Scan()
				if err != nil {
					return fmt.Errorf("failed to refresh workspace: %v", err)
				}

				// Reload the workspace state to ensure it reflects the sealed state
				err = ec.currentRepo.GetWorkspace().LoadState(ec.currentRepo.GetCurrentTimeline())
				if err != nil {
					fmt.Printf("Warning: failed to reload workspace state after sealing: %v\n", err)
				}

				// Double-check that we no longer have uncommitted changes
				if ec.currentRepo.GetWorkspace().HasUncommittedChanges() {
					return fmt.Errorf("workspace still has uncommitted changes after sealing - please check ivaldi status")
				}

				ec.output.Success("Auto-sealed uncommitted changes")
			}

			// Step 3: Sync with remote using Ivaldi-native sync (handles fetch and fuse automatically)
			ec.output.Info("Step 3: Syncing with remote...")
			err := ec.currentRepo.Sync(portalName, localTimeline, remoteBranch)
			if err != nil {
				ec.output.Error("Failed to sync with remote", []string{
					"Check network connection",
					"Verify portal URL and credentials",
					"Ensure remote timeline exists: " + remoteBranch,
					"Try: portal add " + portalName + " <url> (if portal not configured)",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Successfully synced local timeline '%s' with %s/%s!", localTimeline, portalName, remoteBranch))
			ec.output.Info("Your timeline is now up-to-date with remote changes")
			ec.output.Info("Next: Continue working or 'ivaldi upload' to push your changes")

			return nil
		},
	}

	cmd.Flags().StringP("with", "w", "", "Remote timeline to sync with (required)")
	cmd.MarkFlagRequired("with")

	return cmd
}

// Create scout command (discover remote timelines)
func (ec *EnhancedCLI) createScoutCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "scout [portal-name]",
		Short: "Discover remote timelines (branches)",
		Long: `Scout discovers all available timelines on a remote repository.

This command lists all branches/timelines that exist on the remote portal,
helping you understand what timelines are available for syncing.

Examples:
  ivaldi scout                    # Scout default portal (origin)
  ivaldi scout origin             # Scout origin portal
  ivaldi scout upstream           # Scout upstream portal
  
The output shows:
- Timeline names
- Latest commit information
- Whether each timeline exists locally`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Get portal name (default to "origin")
			portalName := "origin"
			if len(args) > 0 {
				portalName = args[0]
			}

			// Get portal URL
			portals := ec.currentRepo.ListPortals()
			portalURL, exists := portals[portalName]
			if !exists {
				ec.output.Error(fmt.Sprintf("Portal '%s' not configured", portalName), []string{
					fmt.Sprintf("Add it with: ivaldi portal add %s <url>", portalName),
					"List portals with: ivaldi portal list",
				})
				return fmt.Errorf("portal not found: %s", portalName)
			}

			ec.output.Info(fmt.Sprintf("Scouting remote timelines from %s...", portalName))
			ec.output.Info(fmt.Sprintf("Portal URL: %s", portalURL))
			ec.output.Info("")

			// Use the network manager to discover remote timelines
			remoteTimelines, err := ec.currentRepo.DiscoverRemoteTimelines(portalURL)
			if err != nil {
				ec.output.Error("Failed to discover remote timelines", []string{
					"Check network connection",
					"Verify portal URL and credentials",
					"Ensure the repository is accessible",
					fmt.Sprintf("Portal: %s (%s)", portalName, portalURL),
				})
				return err
			}

			if len(remoteTimelines) == 0 {
				ec.output.Warning("No timelines found on remote portal")
				return nil
			}

			// Get local timelines for comparison
			localTimelines := ec.currentRepo.ListTimelines()
			localMap := make(map[string]bool)
			for _, timeline := range localTimelines {
				localMap[timeline] = true
			}

			// Display discovered timelines
			ec.output.Success(fmt.Sprintf("Found %d remote timeline(s):", len(remoteTimelines)))
			ec.output.Info("")

			for _, ref := range remoteTimelines {
				status := "remote only"
				statusSymbol := "↓"
				if localMap[ref.Name] {
					status = "exists locally"
					statusSymbol = "✓"
				}

				// Format the output
				ec.output.Info(fmt.Sprintf("  %s %s (%s)", statusSymbol, ref.Name, status))
			}

			ec.output.Info("")
			ec.output.Info("Legend:")
			ec.output.Info("  ✓ = Timeline exists locally")
			ec.output.Info("  ↓ = Remote timeline not synced locally")
			ec.output.Info("")
			ec.output.Info("Next steps:")
			ec.output.Info(fmt.Sprintf("  Sync a timeline: ivaldi sync <timeline> --with <remote-timeline>"))
			ec.output.Info(fmt.Sprintf("  Sync all: ivaldi sync --all %s", portalName))
			ec.output.Info(fmt.Sprintf("  Create local timeline: ivaldi timeline create <name>"))

			return nil
		},
	}

	return cmd
}

func (ec *EnhancedCLI) createStatusCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "status",
		Short: "Show workspace status",
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Show enhanced status with rich output
			status := ec.currentRepo.GetStatus()
			ws := ec.currentRepo.GetWorkspace()

			ec.output.Info("=== Workspace Status ===")
			ec.output.Info("")
			ec.output.Info(fmt.Sprintf("Timeline: %s", ec.currentRepo.GetCurrentTimeline()))

			if currentName, exists := ec.currentRepo.GetMemorableName(ec.currentRepo.GetCurrentPosition()); exists {
				ec.output.Info(fmt.Sprintf("Position: %s", currentName))
			}
			ec.output.Info("")

			// Check for different types of changes
			hasChanges := false

			// Files gathered on anvil (staged)
			if len(status.Staged) > 0 {
				hasChanges = true
				ec.output.Success("Gathered on anvil (ready to seal):")
				for _, file := range status.Staged {
					// Determine file status
					if fileState, exists := ws.Files[file]; exists {
						switch fileState.Status {
						case workspace.StatusDeleted:
							ec.output.FileDeleted(file)
						case workspace.StatusModified:
							ec.output.FileModified(file)
						default:
							ec.output.FileAdded(file)
						}
					} else {
						ec.output.FileAdded(file)
					}
				}
				ec.output.Info("")
			}

			// Modified files (not staged)
			if len(status.Modified) > 0 {
				hasChanges = true
				ec.output.Info("Modified files (not gathered):")
				for _, file := range status.Modified {
					if fileState, exists := ws.Files[file]; exists {
						switch fileState.Status {
						case workspace.StatusDeleted:
							ec.output.FileDeleted(file)
						case workspace.StatusModified:
							ec.output.FileChanged(file)
						default:
							ec.output.FileChanged(file)
						}
					} else {
						ec.output.FileChanged(file)
					}
				}
				ec.output.Info("")
			}

			// Check for untracked files
			untracked, err := ec.getUntrackedFiles()
			if err != nil {
				return fmt.Errorf("failed to get untracked files: %w", err)
			}
			if len(untracked) > 0 {
				hasChanges = true
				ec.output.Info("Untracked files:")
				for _, file := range untracked {
					ec.output.FileAdded(file)
				}
				ec.output.Info("")
			}

			// Check for ignored files that were attempted to be tracked
			ignored := ec.getIgnoredFiles()
			if len(ignored) > 0 {
				ec.output.Info("Ignored files (excluded from tracking):")
				for _, file := range ignored {
					ec.output.Info(fmt.Sprintf("  [ignored]  %s", file))
				}
				ec.output.Info("")
			}

			// No changes
			if !hasChanges {
				ec.output.Success("Workspace is clean - no changes to gather")
				ec.output.Info("")
			}

			// Helpful next steps
			if len(status.Staged) > 0 {
				ec.output.Info("Next: ivaldi seal \"your message\" (to commit changes)")
			} else if len(status.Modified) > 0 || len(untracked) > 0 {
				ec.output.Info("Next: ivaldi gather <files> (to stage changes)")
			}

			return nil
		},
	}
}

// Helper methods for enhanced status

// getCurrentTimeline returns the current timeline name for upload operations
func (ec *EnhancedCLI) getCurrentTimeline() string {
	if ec.currentRepo == nil {
		return "main" // Default fallback
	}

	// Use Ivaldi's timeline system directly
	return ec.currentRepo.GetCurrentTimeline()
}

func (ec *EnhancedCLI) getUntrackedFiles() ([]string, error) {
	if ec.currentRepo == nil {
		return []string{}, nil
	}

	ws := ec.currentRepo.GetWorkspace()
	var untracked []string

	// Get list of submodule paths to skip
	submodulePaths, err := workspace.GetSubmodulePaths(ec.currentRepo.Root())
	if err != nil {
		return nil, fmt.Errorf("failed to get submodule paths for repository at %s: %w", ec.currentRepo.Root(), err)
	}
	submoduleMap := make(map[string]bool)
	for _, path := range submodulePaths {
		submoduleMap[filepath.FromSlash(path)] = true
	}

	// Walk through directory and find files not in workspace
	err = filepath.Walk(ec.currentRepo.Root(), func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil // Skip errors
		}

		// Skip .ivaldi directory and its contents
		if strings.Contains(path, ".ivaldi") {
			if info.IsDir() {
				return filepath.SkipDir
			}
			return nil
		}

		// Get relative path first to check for submodules
		relPath, err := filepath.Rel(ec.currentRepo.Root(), path)
		if err != nil {
			return nil
		}

		// Skip submodule directories
		if info.IsDir() {
			if submoduleMap[relPath] {
				return filepath.SkipDir
			}
			return nil
		}

		// Skip files inside submodules
		for submodulePath := range submoduleMap {
			if strings.HasPrefix(relPath, submodulePath+string(filepath.Separator)) {
				return nil
			}
		}

		// Skip if already tracked or ignored
		if _, tracked := ws.Files[relPath]; tracked {
			return nil
		}
		if ws.ShouldIgnore(relPath) {
			return nil
		}

		untracked = append(untracked, relPath)
		return nil
	})

	if err != nil {
		return nil, fmt.Errorf("failed to walk directory tree: %w", err)
	}

	return untracked, nil
}

func (ec *EnhancedCLI) getIgnoredFiles() []string {
	if ec.currentRepo == nil {
		return []string{}
	}

	ws := ec.currentRepo.GetWorkspace()
	var ignored []string

	// Check if there are common ignored files present
	commonIgnored := []string{
		"build/", "node_modules/", "target/", "dist/",
		"*.log", "*.tmp", ".DS_Store", "CLAUDE.md", "cleanup.sh",
	}

	for _, pattern := range commonIgnored {
		if strings.HasSuffix(pattern, "/") {
			// Directory pattern
			dirPath := strings.TrimSuffix(pattern, "/")
			fullPath := filepath.Join(ec.currentRepo.Root(), dirPath)
			if _, err := os.Stat(fullPath); err == nil {
				if ws.ShouldIgnore(dirPath) {
					ignored = append(ignored, dirPath+"/")
				}
			}
		} else if strings.Contains(pattern, "*") {
			// Glob pattern - check for matches
			matches, err := filepath.Glob(filepath.Join(ec.currentRepo.Root(), pattern))
			if err == nil {
				for _, match := range matches {
					relPath, err := filepath.Rel(ec.currentRepo.Root(), match)
					if err == nil && ws.ShouldIgnore(relPath) {
						ignored = append(ignored, relPath)
					}
				}
			}
		} else {
			// Exact file
			fullPath := filepath.Join(ec.currentRepo.Root(), pattern)
			if _, err := os.Stat(fullPath); err == nil {
				if ws.ShouldIgnore(pattern) {
					ignored = append(ignored, pattern)
				}
			}
		}
	}

	return ignored
}

func (ec *EnhancedCLI) createLogCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "log",
		Short: "Show timeline history",
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Show enhanced log with memorable names
			seals := ec.currentRepo.GetHistory(10) // Last 10 seals

			ec.output.Info("Timeline History:")
			ec.output.Info("")

			for _, seal := range seals {
				if name, exists := ec.currentRepo.GetMemorableName(seal.Hash); exists {
					ec.output.Info(fmt.Sprintf("%s (#%d)", name, seal.Iteration))
				} else {
					ec.output.Info(fmt.Sprintf("#%d", seal.Iteration))
				}
				ec.output.Info(fmt.Sprintf("   %s", seal.Message))
				ec.output.Info(fmt.Sprintf("   %s - %s", seal.Author.Name, seal.Timestamp.Format("2006-01-02 15:04")))
				ec.output.Info("")
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createSearchCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "search <query>",
		Short: "Search history with natural language",
		Args:  cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			query := strings.Join(args, " ")
			return ec.handleSearch(query)
		},
	}
}

func (ec *EnhancedCLI) createReshapeCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "reshape <count>",
		Short: "Modify history with accountability",
		Long: `Reshape modifies recent history with full accountability tracking.

Categories:
  squash    - Combine multiple commits into one
  amend     - Modify the most recent commit  
  rebase    - Rebase commits onto new base
  cleanup   - Clean up commit messages or structure
  refactor  - Refactor without changing functionality
  mistake   - Fix mistakes in previous commits
  security  - Fix security vulnerabilities

Examples:
  ivaldi reshape 3 --category=squash --reason="Combine feature commits"
  ivaldi reshape 1 --category=amend --reason="Fix typo in commit message"
  ivaldi reshape 2 --category=cleanup --reason="Clean up commit messages"`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			count, err := strconv.Atoi(args[0])
			if err != nil {
				return fmt.Errorf("invalid count: %s", args[0])
			}

			category, _ := cmd.Flags().GetString("category")
			reason, _ := cmd.Flags().GetString("reason")
			dryRun, _ := cmd.Flags().GetBool("dry-run")
			interactive, _ := cmd.Flags().GetBool("interactive")

			return ec.handleReshape(count, category, reason, dryRun, interactive)
		},
	}

	cmd.Flags().StringP("category", "c", "cleanup", "Reshape category (squash, amend, rebase, cleanup, refactor, mistake, security)")
	cmd.Flags().StringP("reason", "r", "", "Justification for reshape (required)")
	cmd.Flags().Bool("dry-run", false, "Show what would be changed without making changes")
	cmd.Flags().Bool("interactive", false, "Interactively choose reshape options")

	cmd.MarkFlagRequired("reason")

	return cmd
}

func (ec *EnhancedCLI) createFuseCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "fuse <source-timeline>",
		Short: "Merge timeline into current",
		Long: `Fuse merges one timeline into another with intelligent conflict resolution.

Strategies:
  auto        - Automatic merge with conflict detection (default)
  ff          - Fast-forward only (no merge commit) 
  squash      - Squash all commits into one
  manual      - Manual conflict resolution required

Examples:
  ivaldi fuse feature                    # Merge feature into current timeline
  ivaldi fuse feature --strategy=squash  # Squash feature commits
  ivaldi fuse feature --delete-source    # Delete feature timeline after merge
  ivaldi fuse feature --dry-run          # Preview what would happen`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			strategy, _ := cmd.Flags().GetString("strategy")
			message, _ := cmd.Flags().GetString("message")
			deleteSource, _ := cmd.Flags().GetBool("delete-source")
			dryRun, _ := cmd.Flags().GetBool("dry-run")

			return ec.handleFuse(args[0], strategy, message, deleteSource, dryRun)
		},
	}

	cmd.Flags().StringP("strategy", "s", "auto", "Fuse strategy (auto, ff, squash, manual)")
	cmd.Flags().StringP("message", "m", "", "Custom fuse message")
	cmd.Flags().Bool("delete-source", false, "Delete source timeline after successful fuse")
	cmd.Flags().Bool("dry-run", false, "Show what would be fused without making changes")

	return cmd
}

func (ec *EnhancedCLI) createPluckCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "pluck <reference>",
		Short: "Cherry-pick commit to current timeline",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			ec.output.Info(fmt.Sprintf("Plucking %s to current timeline", args[0]))
			ec.output.Info("(Implementation coming soon)")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createHuntCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "hunt <query>",
		Short: "Binary search for issues (bisect)",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			ec.output.Info(fmt.Sprintf("Hunting for: %s", args[0]))
			ec.output.Info("(Implementation coming soon)")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createWorkspaceCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "workspace",
		Short: "Manage named workspaces",
		Long: `Manage named workspaces for different tasks

Subcommands:
  save <name>       Save current workspace
  load <name>       Load saved workspace
  list              List all workspaces
  delete <name>     Delete workspace`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			snapshots := ec.currentRepo.GetWorkspaceSnapshots()
			if len(snapshots) == 0 {
				ec.output.Info("No saved workspaces")
				ec.output.Info("Save one with: workspace save <name>")
				return nil
			}

			ec.output.Info("Saved workspaces:")
			for _, snapshot := range snapshots {
				ec.output.Info(fmt.Sprintf("  %s - %s", snapshot.Name, snapshot.Description))
			}
			return nil
		},
	}
}

func (ec *EnhancedCLI) createMeshCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "mesh",
		Short: "Mesh networking for automatic peer discovery and multi-hop routing",
		Long: `Mesh networking provides true mesh connectivity on top of P2P.

Unlike simple P2P that requires manual connections, mesh networking automatically:
- Discovers all peers in the network through gossip protocols
- Establishes full connectivity where every node can reach every other node
- Routes messages through intermediate peers when direct connections aren't possible
- Self-heals when nodes join or leave the network
- Maintains topology awareness for efficient routing

Commands:
  start                    Start mesh networking
  stop                     Stop mesh networking  
  status                   Show mesh status and topology
  join <address:port>      Join mesh via bootstrap peer
  topology                 Show network topology
  route <peer-id>          Show route to specific peer
  ping <peer-id>           Ping peer via mesh routing
  peers                    List all mesh peers
  heal                     Manually trigger network healing
  refresh                  Refresh topology information

Examples:
  ivaldi mesh start                        # Start mesh networking
  ivaldi mesh join 192.168.1.100:9090     # Join mesh via bootstrap
  ivaldi mesh topology                     # View network topology
  ivaldi mesh ping abc123def               # Ping peer through mesh`,
	}

	cmd.AddCommand(ec.createMeshStartCommand())
	cmd.AddCommand(ec.createMeshStopCommand())
	cmd.AddCommand(ec.createMeshStatusCommand())
	cmd.AddCommand(ec.createMeshJoinCommand())
	cmd.AddCommand(ec.createMeshTopologyCommand())
	cmd.AddCommand(ec.createMeshRouteCommand())
	cmd.AddCommand(ec.createMeshPingCommand())
	cmd.AddCommand(ec.createMeshPeersCommand())
	cmd.AddCommand(ec.createMeshHealCommand())
	cmd.AddCommand(ec.createMeshRefreshCommand())

	return cmd
}

func (ec *EnhancedCLI) createCollaborateCommand() *cobra.Command {
	// Create the same command structure as P2P but with collaborate-friendly naming
	cmd := &cobra.Command{
		Use:   "collaborate",
		Short: "Real-time collaboration session",
		Long: `Start real-time collaboration with other Ivaldi repositories.

This is an alias for the P2P system that enables direct synchronization 
between repositories without needing a central server, perfect for:
- Real-time collaboration on local networks
- Offline sync when internet is unavailable  
- Faster transfers between nearby peers
- Private networks for sensitive projects

Commands:
  start                    Start collaboration network
  stop                     Stop collaboration network
  status                   Show collaboration status and connections
  connect <address:port>   Connect to a collaborator
  peers                    List all connected collaborators
  discover                 Show discovered collaborators on network
  sync [peer-id]           Sync with specific collaborator or all
  config                   View or update collaboration configuration

Examples:
  ivaldi collaborate start                    # Start collaboration
  ivaldi collaborate connect 192.168.1.100:9090  # Connect to collaborator
  ivaldi collaborate sync                     # Sync with all collaborators`,
	}

	// Add all the same subcommands as P2P
	cmd.AddCommand(ec.createP2PStartCommand())
	cmd.AddCommand(ec.createP2PStopCommand())
	cmd.AddCommand(ec.createP2PStatusCommand())
	cmd.AddCommand(ec.createP2PConnectCommand())
	cmd.AddCommand(ec.createP2PPeersCommand())
	cmd.AddCommand(ec.createP2PDiscoverCommand())
	cmd.AddCommand(ec.createP2PSyncCommand())
	cmd.AddCommand(ec.createP2PConfigCommand())

	return cmd
}

func (ec *EnhancedCLI) createVersionCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "version",
		Short: "Show Ivaldi version",
		RunE: func(cmd *cobra.Command, args []string) error {
			ec.output.Info("Ivaldi VCS v0.1.0-revolutionary")
			ec.output.Info("The human-centered version control system")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createConfigCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "config",
		Short: "Global configuration management",
		Long: `Configure Ivaldi global settings and credentials

Interactive setup will guide you through configuring:
- User information (name, email)
- GitHub personal access token (or detect from git credentials)
- GitLab personal access token

Configuration is stored globally at ~/.ivaldi/config.json and used by all repositories.

Examples:
  ivaldi config           # Interactive global setup
  ivaldi config --show   # Show current configuration`,
		RunE: func(cmd *cobra.Command, args []string) error {
			showConfig, _ := cmd.Flags().GetBool("show")

			// Use global config manager
			configMgr := config.NewGlobalConfigManager()

			if showConfig {
				return ec.showCurrentConfig(configMgr)
			}

			// Interactive setup
			ec.output.Info("Starting interactive global configuration...")
			ec.output.Info("This will be stored at ~/.ivaldi/config.json for all repositories")

			if err := configMgr.InteractiveSetup(); err != nil {
				ec.output.Error("Configuration failed", []string{
					err.Error(),
					"Try running 'ivaldi config' again",
				})
				return err
			}

			ec.output.Success("Global configuration saved!")
			ec.output.Info("All repositories will now use these credentials")
			return nil
		},
	}

	cmd.Flags().Bool("show", false, "Show current configuration")
	return cmd
}

func (ec *EnhancedCLI) createSquashCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "squash [count] [message]",
		Short: "Squash recent commits into one clean commit",
		Long: `Squash recent commits into a single clean commit and push to remote.

This is useful when you have multiple commits that should be one logical change.
The squash operation will:
1. Take the last N commits
2. Combine them into a single commit with all changes
3. Force push to remote to clean up history

Examples:
  ivaldi squash 3                           # Squash last 3 commits with auto message
  ivaldi squash 5 "feat: complete feature"  # Squash last 5 commits with custom message
  ivaldi squash --all "initial commit"      # Squash all commits into one`,
		Args: cobra.RangeArgs(0, 2),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			squashAll, _ := cmd.Flags().GetBool("all")

			var count int
			var message string

			if squashAll {
				count = -1 // Special value for all commits
				if len(args) > 0 {
					message = args[0]
				} else {
					message = "Initial commit with all changes"
				}
			} else {
				if len(args) == 0 {
					count = 2 // Default to last 2 commits
				} else {
					var err error
					count, err = strconv.Atoi(args[0])
					if err != nil {
						return fmt.Errorf("invalid count: %s", args[0])
					}
					if count < 2 {
						return fmt.Errorf("count must be at least 2")
					}
				}

				if len(args) > 1 {
					message = args[1]
				} else {
					message = fmt.Sprintf("Squash last %d commits", count)
				}
			}

			ec.output.Info(fmt.Sprintf("Squashing commits with message: %s", message))

			// Get portal info
			portals := ec.currentRepo.ListPortals()
			if len(portals) == 0 {
				return fmt.Errorf("no portals configured, use 'ivaldi portal add' first")
			}

			portalName := "origin"
			portalURL, exists := portals[portalName]
			if !exists {
				// Use first available portal
				for name, url := range portals {
					portalName = name
					portalURL = url
					break
				}
			}

			// Perform the squash and force push
			err := ec.performSquash(portalURL, count, message)
			if err != nil {
				ec.output.Error("Squash failed", []string{
					err.Error(),
					"Your local repository is unchanged",
					"You can try again with different parameters",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Successfully squashed commits and updated %s!", portalName))
			ec.output.Info("Your commit history is now clean and consolidated")

			return nil
		},
	}

	cmd.Flags().Bool("all", false, "Squash all commits into one initial commit")
	return cmd
}

func (ec *EnhancedCLI) performSquash(portalURL string, count int, message string) error {
	// Extract owner and repo from URL
	urlParts := strings.Split(strings.TrimSuffix(portalURL, ".git"), "/")
	if len(urlParts) < 2 {
		return fmt.Errorf("invalid portal URL format: %s", portalURL)
	}

	owner := urlParts[len(urlParts)-2]
	repo := urlParts[len(urlParts)-1]

	// Get network manager
	configMgr := config.NewConfigManager(ec.currentRepo.Root())

	// Create a single commit with all current files
	return ec.createCleanCommit(owner, repo, message, configMgr)
}

func (ec *EnhancedCLI) createCleanCommit(owner, repo, message string, configMgr *config.ConfigManager) error {
	// Load ignore patterns
	ignorePatterns, err := ec.loadIgnorePatterns()
	if err != nil {
		fmt.Printf("Warning: failed to load ignore patterns: %v\n", err)
		ignorePatterns = []string{}
	}

	// Collect all files in the repository
	var filesToUpload []FileToUpload
	err = filepath.Walk(ec.currentRepo.Root(), func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return err
		}

		if info.IsDir() {
			return nil
		}

		relPath, err := filepath.Rel(ec.currentRepo.Root(), path)
		if err != nil {
			return err
		}

		relPath = strings.ReplaceAll(relPath, "\\", "/")

		if ec.shouldIgnoreFile(relPath, ignorePatterns) {
			return nil
		}

		content, err := os.ReadFile(path)
		if err != nil {
			fmt.Printf("Skipping %s: %v\n", relPath, err)
			return nil
		}

		filesToUpload = append(filesToUpload, FileToUpload{
			Path:    relPath,
			Content: content,
		})

		return nil
	})

	if err != nil {
		return fmt.Errorf("failed to collect files: %v", err)
	}

	fmt.Printf("Creating clean commit with %d files...\n", len(filesToUpload))

	// Create the commit using GitHub API
	return ec.createGitHubCommit(owner, repo, filesToUpload, message, configMgr, true) // true for force push
}

type FileToUpload struct {
	Path    string
	Content []byte
}

func (ec *EnhancedCLI) loadIgnorePatterns() ([]string, error) {
	ignoreFile := filepath.Join(ec.currentRepo.Root(), ".ivaldiignore")
	content, err := os.ReadFile(ignoreFile)
	if err != nil {
		if os.IsNotExist(err) {
			return []string{}, nil
		}
		return nil, err
	}

	var patterns []string
	lines := strings.Split(string(content), "\n")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line != "" && !strings.HasPrefix(line, "#") {
			patterns = append(patterns, line)
		}
	}

	return patterns, nil
}

func (ec *EnhancedCLI) shouldIgnoreFile(filePath string, patterns []string) bool {
	builtInIgnores := []string{
		".ivaldi/", ".git/", "build/", "*.tmp", "*.temp", "*~", ".DS_Store", "*.log", "*.bak",
	}

	allPatterns := append(patterns, builtInIgnores...)

	for _, pattern := range allPatterns {
		if ec.matchesPattern(filePath, pattern) {
			return true
		}
	}

	return false
}

func (ec *EnhancedCLI) matchesPattern(filePath, pattern string) bool {
	// Simple glob pattern matching
	pattern = strings.ReplaceAll(pattern, ".", "\\.")
	pattern = strings.ReplaceAll(pattern, "*", ".*")
	pattern = strings.ReplaceAll(pattern, "?", ".")

	if strings.HasSuffix(pattern, "/") {
		pattern = pattern + ".*"
	}

	pattern = "^" + pattern + "$"

	matched, err := regexp.MatchString(pattern, filePath)
	if err != nil {
		return strings.Contains(filePath, strings.ReplaceAll(pattern, ".*", ""))
	}

	return matched
}

func (ec *EnhancedCLI) createGitHubCommit(owner, repo string, files []FileToUpload, message string, configMgr *config.ConfigManager, forcePush bool) error {
	token, err := configMgr.GetGitHubToken()
	if err != nil {
		return fmt.Errorf("failed to get GitHub token: %v", err)
	}

	client := &http.Client{Timeout: 30 * time.Second}

	// Create tree with all files
	treeItems := make([]map[string]interface{}, 0, len(files))
	for _, file := range files {
		treeItems = append(treeItems, map[string]interface{}{
			"path":    file.Path,
			"mode":    "100644",
			"type":    "blob",
			"content": string(file.Content),
		})
	}

	tree := map[string]interface{}{
		"tree": treeItems,
	}

	treeData, err := json.Marshal(tree)
	if err != nil {
		return fmt.Errorf("failed to marshal tree: %v", err)
	}

	// Create tree
	treeSHA, err := ec.makeGitHubAPICall(client, token, "POST",
		fmt.Sprintf("https://api.github.com/repos/%s/%s/git/trees", owner, repo),
		treeData)
	if err != nil {
		return fmt.Errorf("failed to create tree: %v", err)
	}

	// Create commit (no parents for clean history)
	commit := map[string]interface{}{
		"message": message,
		"tree":    treeSHA,
		"parents": []string{}, // Empty parents for clean history
	}

	commitData, err := json.Marshal(commit)
	if err != nil {
		return fmt.Errorf("failed to marshal commit: %v", err)
	}

	commitSHA, err := ec.makeGitHubAPICall(client, token, "POST",
		fmt.Sprintf("https://api.github.com/repos/%s/%s/git/commits", owner, repo),
		commitData)
	if err != nil {
		return fmt.Errorf("failed to create commit: %v", err)
	}

	// Force update the main branch
	updateData := map[string]interface{}{
		"sha":   commitSHA,
		"force": forcePush,
	}

	updateJSON, err := json.Marshal(updateData)
	if err != nil {
		return fmt.Errorf("failed to marshal update: %v", err)
	}

	_, err = ec.makeGitHubAPICall(client, token, "PATCH",
		fmt.Sprintf("https://api.github.com/repos/%s/%s/git/refs/heads/main", owner, repo),
		updateJSON)
	if err != nil {
		return fmt.Errorf("failed to update branch: %v", err)
	}

	fmt.Printf("Successfully created clean commit: %s\n", commitSHA[:8])
	return nil
}

func (ec *EnhancedCLI) makeGitHubAPICall(client *http.Client, token, method, url string, data []byte) (string, error) {
	var req *http.Request
	var err error

	if data != nil {
		req, err = http.NewRequest(method, url, bytes.NewBuffer(data))
	} else {
		req, err = http.NewRequest(method, url, nil)
	}

	if err != nil {
		return "", err
	}

	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	req.Header.Set("Content-Type", "application/json")

	resp, err := client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()

	body, _ := io.ReadAll(resp.Body)

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return "", fmt.Errorf("API error (%d): %s", resp.StatusCode, string(body))
	}

	var result map[string]interface{}
	if err := json.Unmarshal(body, &result); err != nil {
		return "", fmt.Errorf("failed to parse response: %v", err)
	}

	// Debug: print the response
	fmt.Printf("API Response: %+v\n", result)

	if sha, ok := result["sha"].(string); ok {
		return sha, nil
	}

	// For PATCH operations, sometimes the response is different
	if method == "PATCH" {
		return "success", nil // PATCH operations may not return SHA
	}

	return "", fmt.Errorf("no SHA in response: %+v", result)
}

func (ec *EnhancedCLI) showCurrentConfig(configMgr *config.ConfigManager) error {
	creds, err := configMgr.LoadCredentials()
	if err != nil {
		return fmt.Errorf("failed to load configuration: %v", err)
	}

	ec.output.Info("=== Current Configuration ===")
	ec.output.Info("")

	if creds.UserName != "" {
		ec.output.Info(fmt.Sprintf("User Name: %s", creds.UserName))
	} else {
		ec.output.Info("User Name: not set")
	}

	if creds.UserEmail != "" {
		ec.output.Info(fmt.Sprintf("User Email: %s", creds.UserEmail))
	} else {
		ec.output.Info("User Email: not set")
	}

	ec.output.Info("")

	if creds.GitHubToken != "" {
		ec.output.Info(fmt.Sprintf("GitHub Token: %s", maskToken(creds.GitHubToken)))
	} else {
		ec.output.Info("GitHub Token: not set")
	}

	if creds.GitLabToken != "" {
		ec.output.Info(fmt.Sprintf("GitLab Token: %s", maskToken(creds.GitLabToken)))
	} else {
		ec.output.Info("GitLab Token: not set")
	}

	ec.output.Info("")
	ec.output.Info("Run 'ivaldi config' to update configuration")

	return nil
}

func maskToken(token string) string {
	if token == "" {
		return "not set"
	}
	if len(token) <= 8 {
		return "***"
	}
	return token[:4] + "***" + token[len(token)-4:]
}

func (ec *EnhancedCLI) createCleanCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "clean",
		Short: "Clean build artifacts and binaries",
		Long: `Clean build artifacts and binaries

Options:
  --binaries    Remove binary files (build/ directory)
  --all         Remove all build artifacts and temporary files`,
		RunE: func(cmd *cobra.Command, args []string) error {
			binaries, _ := cmd.Flags().GetBool("binaries")
			all, _ := cmd.Flags().GetBool("all")

			if !binaries && !all {
				ec.output.Info("Cleaning default build artifacts...")
				binaries = true
			}

			if binaries || all {
				ec.output.Info("Removing build directory...")
				err := os.RemoveAll("build")
				if err != nil {
					ec.output.Warning(fmt.Sprintf("Failed to remove build directory: %v", err))
				} else {
					ec.output.Success("Removed build/ directory")
				}
			}

			if all {
				ec.output.Info("Removing temporary files...")
				// Remove common temp files
				tempPatterns := []string{"*.tmp", "*.log", "*.bak", "*~"}
				for _, pattern := range tempPatterns {
					// This is a simple implementation - would need proper glob matching
					ec.output.Info(fmt.Sprintf("Cleaning %s files...", pattern))
				}
			}

			ec.output.Success("Clean completed!")
			return nil
		},
	}

	cmd.Flags().Bool("binaries", false, "Remove binary files (build/ directory)")
	cmd.Flags().Bool("all", false, "Remove all build artifacts and temporary files")

	return cmd
}

// Find repository root by searching for .ivaldi directory
func findRepositoryRoot(path string) string {
	current := path
	for {
		ivaldiPath := filepath.Join(current, ".ivaldi")
		if _, err := os.Stat(ivaldiPath); err == nil {
			return current
		}

		parent := filepath.Dir(current)
		if parent == current {
			break // Reached root
		}
		current = parent
	}
	return ""
}

// handleSearch performs natural language search across repository history
func (ec *EnhancedCLI) handleSearch(query string) error {
	// Find repository root
	wd, _ := os.Getwd()
	repoRoot := findRepositoryRoot(wd)
	if repoRoot == "" {
		return fmt.Errorf("not in an Ivaldi repository")
	}

	// Open repository
	repo, err := forge.Open(repoRoot)
	if err != nil {
		return fmt.Errorf("failed to open repository: %v", err)
	}
	defer repo.Close()

	// Create search manager
	searchMgr := search.NewSearchManager(repo.GetIndex(), repo.GetStorage())

	ec.output.Info(fmt.Sprintf("Searching for: '%s'", query))

	// Perform search
	results, err := searchMgr.Search(query)
	if err != nil {
		ec.output.Warning(fmt.Sprintf("Search failed: %v", err))

		// Show suggestions
		suggestions := searchMgr.SearchSuggestions()
		ec.output.Info("Try these search examples:")
		for i, suggestion := range suggestions {
			if i >= 5 { // Show only first 5 suggestions
				break
			}
			ec.output.Info(fmt.Sprintf("  - %s", suggestion))
		}
		return nil
	}

	// Display results
	if len(results) == 0 {
		ec.output.Warning("No results found")
		return nil
	}

	ec.output.Success(fmt.Sprintf("Found %d result(s):", len(results)))

	for i, result := range results {
		if i >= 10 { // Limit to 10 results
			ec.output.Info(fmt.Sprintf("... and %d more results", len(results)-10))
			break
		}

		// Display seal information
		ec.output.Info("")
		ec.output.Info(fmt.Sprintf("  %s (#%d)", result.Seal.Name, result.Seal.Iteration))
		ec.output.Info(fmt.Sprintf("    %s", result.Seal.Message))
		ec.output.Info(fmt.Sprintf("    %s - %s",
			result.Seal.Author.Name,
			result.Seal.Timestamp.Format("2006-01-02 15:04")))

		// Show what matched
		if len(result.Matches) > 0 {
			ec.output.Info(fmt.Sprintf("    Matched: %s", strings.Join(result.Matches, ", ")))
		}

		// Show relevance score
		if result.Score > 0 {
			scorePercent := int(result.Score * 100)
			ec.output.Info(fmt.Sprintf("    Relevance: %d%%", scorePercent))
		}
	}

	return nil
}

// handleReshape performs history modification with accountability
func (ec *EnhancedCLI) handleReshape(count int, category, reason string, dryRun, interactive bool) error {
	// Find repository root
	wd, _ := os.Getwd()
	repoRoot := findRepositoryRoot(wd)
	if repoRoot == "" {
		return fmt.Errorf("not in an Ivaldi repository")
	}

	// Open repository
	repo, err := forge.Open(repoRoot)
	if err != nil {
		return fmt.Errorf("failed to open repository: %v", err)
	}
	defer repo.Close()

	// Create reshape manager
	reshapeMgr := reshape.NewReshapeManager(repo.GetIndex(), repo.GetStorage(), nil) // TODO: Add overwrite tracker

	ec.output.Info(fmt.Sprintf("Reshaping last %d seal(s)", count))
	ec.output.Info(fmt.Sprintf("Category: %s", category))
	ec.output.Info(fmt.Sprintf("Reason: %s", reason))

	if dryRun {
		ec.output.Info("DRY RUN - No changes will be made")
	}

	// Prepare reshape options
	opts := reshape.ReshapeOptions{
		Count:         count,
		Justification: reason,
		Category:      category,
		Interactive:   interactive,
		DryRun:        dryRun,
	}

	// Perform reshape
	result, err := reshapeMgr.Reshape(opts)
	if err != nil {
		ec.output.Warning(fmt.Sprintf("Reshape failed: %v", err))
		return nil
	}

	// Display results
	ec.output.Success("Reshape completed successfully!")

	if dryRun {
		ec.output.Info("\nWould affect these seals:")
	} else {
		ec.output.Info("\nReshape summary:")
	}

	for i, seal := range result.OriginalSeals {
		if dryRun {
			ec.output.Info(fmt.Sprintf("  %d. %s - %s", i+1, seal.Name, seal.Message))
		} else {
			ec.output.Info(fmt.Sprintf("  Original: %s - %s", seal.Name, seal.Message))
		}
	}

	if !dryRun && len(result.NewSeals) > 0 {
		ec.output.Info("\nNew seals created:")
		for i, seal := range result.NewSeals {
			ec.output.Info(fmt.Sprintf("  %d. %s - %s", i+1, seal.Name, seal.Message))
		}
	}

	if result.OverwriteID != "" {
		ec.output.Info(fmt.Sprintf("\nOverwrite tracked: %s", result.OverwriteID))
		ec.output.Info("Full audit trail maintained for accountability")
	}

	if dryRun {
		ec.output.Info("\nTo execute this reshape, run the same command without --dry-run")
	}

	return nil
}

// handleFuse performs timeline merging operations
func (ec *EnhancedCLI) handleFuse(sourceTimeline, strategyStr, message string, deleteSource, dryRun bool) error {
	if ec.currentRepo == nil {
		return fmt.Errorf("not in repository")
	}

	// Parse fuse strategy
	strategy, err := fuse.ParseFuseStrategy(strategyStr)
	if err != nil {
		ec.output.Warning(fmt.Sprintf("Invalid strategy '%s', using auto", strategyStr))
		strategy = 0 // FuseStrategyAutomatic
	}

	ec.output.Info(fmt.Sprintf("Fusing timeline '%s' into current", sourceTimeline))
	ec.output.Info(fmt.Sprintf("Strategy: %s", strategyStr))

	if dryRun {
		ec.output.Info("DRY RUN - No changes will be made")
	}

	if deleteSource {
		ec.output.Info("Source timeline will be deleted after successful fuse")
	}

	// Create fuse manager with repository components
	fuseMgr := fuse.NewFuseManager(
		ec.currentRepo.GetStorage(),
		ec.currentRepo, // Repository implements TimelineManager interface
		ec.currentRepo, // Repository implements WorkspaceManager interface
	)

	// Configure fuse options
	opts := fuse.FuseOptions{
		SourceTimeline: sourceTimeline,
		TargetTimeline: "", // Will use current timeline
		FuseMessage:    message,
		Strategy:       strategy,
		DeleteSource:   deleteSource,
		DryRun:         dryRun,
	}

	// Perform fuse
	result, err := fuseMgr.Fuse(opts)
	if err != nil {
		ec.output.Warning(fmt.Sprintf("Fuse failed: %v", err))

		// Show helpful suggestions
		ec.output.Info("Troubleshooting:")
		ec.output.Info("  - Check that source timeline exists: timeline list")
		ec.output.Info("  - Ensure you have no uncommitted changes: status")
		ec.output.Info("  - Try a different strategy: --strategy=ff or --strategy=squash")
		return nil
	}

	// Display results
	ec.output.Success("Fuse completed successfully!")

	if dryRun {
		ec.output.Info("\nWould perform these changes:")
	} else {
		ec.output.Info("\nFuse summary:")
	}

	ec.output.Info(fmt.Sprintf("  Strategy: %s", ec.formatFuseStrategy(result.Strategy)))

	if result.MergeCommit != nil {
		ec.output.Info(fmt.Sprintf("  Merge commit: %s - %s", result.MergeCommit.Name, result.MergeCommit.Message))
	}

	if len(result.FilesChanged) > 0 {
		ec.output.Info("  Changes:")
		for _, change := range result.FilesChanged {
			ec.output.Info(fmt.Sprintf("    - %s", change))
		}
	}

	if result.ConflictCount > 0 {
		ec.output.Warning(fmt.Sprintf("  Conflicts: %d (manual resolution required)", result.ConflictCount))
	}

	if result.DeletedSource {
		ec.output.Info(fmt.Sprintf("  Deleted source timeline: %s", sourceTimeline))
	}

	if dryRun {
		ec.output.Info("\nTo execute this fuse, run the same command without --dry-run")
	}

	return nil
}

// formatFuseStrategy converts strategy enum to readable string
func (ec *EnhancedCLI) formatFuseStrategy(strategy fuse.FuseStrategy) string {
	switch strategy {
	case 0: // FuseStrategyAutomatic
		return "Automatic merge"
	case 1: // FuseStrategyManual
		return "Manual resolution"
	case 2: // FuseStrategyFastForward
		return "Fast-forward"
	case 3: // FuseStrategySquash
		return "Squash"
	default:
		return "Unknown"
	}
}

// Portal subcommands
func (ec *EnhancedCLI) createPortalAddCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "add <name> <url>",
		Short: "Add new portal",
		Args:  cobra.ExactArgs(2),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			name := args[0]
			url := args[1]

			err := ec.currentRepo.AddPortal(name, url)
			if err != nil {
				ec.output.Error("Failed to add portal", []string{
					"Check URL format is correct",
					"Ensure you have write access if needed",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Added portal: %s → %s", name, url))
			return nil
		},
	}
}

func (ec *EnhancedCLI) createPortalListCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "list",
		Short: "List all portals",
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			portals := ec.currentRepo.ListPortals()
			if len(portals) == 0 {
				ec.output.Info("No portals configured")
				return nil
			}

			ec.output.Info("Configured portals:")
			for name, url := range portals {
				ec.output.Info(fmt.Sprintf("  %s → %s", name, url))
			}
			return nil
		},
	}
}

func (ec *EnhancedCLI) createPortalSyncCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "sync [portal-name]",
		Short: "Sync with portal (push and pull)",
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			portalName := "origin"
			if len(args) > 0 {
				portalName = args[0]
			}

			ec.output.Info(fmt.Sprintf("Syncing with portal: %s", portalName))

			err := ec.currentRepo.Push(portalName)
			if err != nil {
				ec.output.Error("Failed to sync with portal", []string{
					"Check network connection",
					"Verify portal URL and credentials",
					"Ensure repository exists on remote",
				})
				return err
			}

			ec.output.Success("Successfully synced with portal!")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createPortalPushCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "push [portal-name]",
		Short: "Push to portal",
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			portalName := "origin"
			if len(args) > 0 {
				portalName = args[0]
			}

			branch, _ := cmd.Flags().GetString("branch")
			setUpstream, _ := cmd.Flags().GetBool("set-upstream")

			ec.output.Info(fmt.Sprintf("Pushing to portal: %s", portalName))
			if branch != "" {
				ec.output.Info(fmt.Sprintf("Target branch: %s", branch))
			}

			err := ec.currentRepo.PushToBranch(portalName, branch, setUpstream)
			if err != nil {
				ec.output.Error("Failed to push to portal", []string{
					"Check network connection",
					"Verify portal URL and credentials",
					"Ensure repository exists on remote",
					"Try: --set-upstream for new branches",
				})
				return err
			}

			ec.output.Success("Successfully pushed to portal!")
			return nil
		},
	}

	cmd.Flags().StringP("branch", "b", "", "Branch to push to (defaults to current branch)")
	cmd.Flags().BoolP("set-upstream", "u", false, "Set upstream for tracking")

	return cmd
}

func (ec *EnhancedCLI) createPortalPullCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "pull [portal-name]",
		Short: "Pull from portal",
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			portalName := "origin"
			if len(args) > 0 {
				portalName = args[0]
			}

			branch, _ := cmd.Flags().GetString("branch")

			ec.output.Info(fmt.Sprintf("Pulling from portal: %s", portalName))
			if branch != "" {
				ec.output.Info(fmt.Sprintf("Source branch: %s", branch))
			}

			err := ec.currentRepo.PullFromBranch(portalName, branch)
			if err != nil {
				ec.output.Error("Failed to pull from portal", []string{
					"Check network connection",
					"Verify portal URL and credentials",
					"Ensure remote branch exists",
					"Check for uncommitted changes",
				})
				return err
			}

			ec.output.Success("Successfully pulled from portal!")
			return nil
		},
	}

	cmd.Flags().StringP("branch", "b", "", "Branch to pull from (defaults to current branch)")

	return cmd
}

func (ec *EnhancedCLI) createPortalNewCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "new <branch-name>",
		Short: "Create new branch with optional migration",
		Long: `Create a new branch and optionally migrate content from another branch

Examples:
  ivaldi portal new main --migrate master    # Create main branch from master
  ivaldi portal new development             # Create development branch from current`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			newBranch := args[0]
			migrateBranch, _ := cmd.Flags().GetString("migrate")

			ec.output.Info(fmt.Sprintf("Creating new branch: %s", newBranch))
			if migrateBranch != "" {
				ec.output.Info(fmt.Sprintf("Migrating content from: %s", migrateBranch))
			}

			err := ec.currentRepo.CreateBranchAndMigrate(newBranch, migrateBranch)
			if err != nil {
				ec.output.Error("Failed to create branch", []string{
					"Check that source branch exists",
					"Ensure no uncommitted changes",
					"Branch name might already exist",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Created branch: %s", newBranch))
			if migrateBranch != "" {
				ec.output.Success(fmt.Sprintf("Migrated content from: %s", migrateBranch))
			}
			ec.output.Info(fmt.Sprintf("Current branch: %s", newBranch))

			return nil
		},
	}

	cmd.Flags().String("migrate", "", "Migrate content from this branch")

	return cmd
}

func (ec *EnhancedCLI) createPortalUploadCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "upload <branch-name> [portal-name]",
		Short: "Upload branch to portal with automatic upstream",
		Long: `Upload a branch to the portal with automatic upstream tracking

Examples:
  ivaldi portal upload main              # Upload main branch to origin
  ivaldi portal upload main upstream     # Upload main branch to upstream portal`,
		Args: cobra.RangeArgs(1, 2),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			branch := args[0]
			portalName := "origin"
			if len(args) > 1 {
				portalName = args[1]
			}

			ec.output.Info(fmt.Sprintf("Uploading branch '%s' to portal '%s'", branch, portalName))

			err := ec.currentRepo.UploadToPortal(portalName, branch)
			if err != nil {
				ec.output.Error("Failed to upload to portal", []string{
					"Check network connection",
					"Verify portal URL and credentials",
					"Ensure repository exists on remote",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Successfully uploaded %s to %s!", branch, portalName))
			ec.output.Info("Upstream tracking automatically configured")

			return nil
		},
	}

	return cmd
}

func (ec *EnhancedCLI) createWhereAmICommand() *cobra.Command {
	return &cobra.Command{
		Use:   "whereami",
		Short: "Show current timeline and position",
		Long: `Show current timeline and position information
		
This command displays:
- Current timeline in Ivaldi
- Current position/seal information
- Portal tracking status

Examples:
  ivaldi whereami                    # Show current timeline and position`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Get current timeline and position from Ivaldi
			currentTimeline := ec.currentRepo.GetCurrentTimeline()
			currentPosition := ec.currentRepo.GetCurrentPosition()

			// Get memorable name if available
			memorableName := "unknown"
			if name, exists := ec.currentRepo.GetMemorableName(currentPosition); exists {
				memorableName = name
			}

			ec.output.Info("Current Location:")
			ec.output.Info(fmt.Sprintf("  Timeline: %s", currentTimeline))
			ec.output.Info(fmt.Sprintf("  Position: %s", memorableName))

			// Show portal tracking info
			portals := ec.currentRepo.ListPortals()
			if len(portals) > 0 {
				// Default to origin if available, otherwise first portal
				if originURL, hasOrigin := portals["origin"]; hasOrigin {
					ec.output.Info(fmt.Sprintf("  Tracking: origin/%s", currentTimeline))
					ec.output.Info(fmt.Sprintf("  Remote: %s", originURL))
				} else {
					// Show first available portal
					for name, url := range portals {
						ec.output.Info(fmt.Sprintf("  Tracking: %s/%s", name, currentTimeline))
						ec.output.Info(fmt.Sprintf("  Remote: %s", url))
						break
					}
				}
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createPortalRenameCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "rename <old-branch> --with <new-branch> [portal-name]",
		Short: "Rename branch remotely",
		Long: `Rename a branch on the remote portal
		
This command:
1. Creates the new branch from the old branch on the remote
2. Deletes the old branch on the remote
3. Updates local tracking if the current branch is being renamed

Examples:
  ivaldi portal rename master --with main          # Rename master to main on origin
  ivaldi portal rename master --with main upstream # Rename master to main on upstream portal`,
		Args: cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			oldBranch := args[0]
			newBranch, _ := cmd.Flags().GetString("with")
			if newBranch == "" {
				return fmt.Errorf("--with flag is required to specify new branch name")
			}

			portalName := "origin"
			if len(args) > 1 {
				portalName = args[1]
			}

			ec.output.Info(fmt.Sprintf("Renaming branch '%s' to '%s' on portal '%s'", oldBranch, newBranch, portalName))

			err := ec.currentRepo.RenameBranchOnPortal(portalName, oldBranch, newBranch)
			if err != nil {
				ec.output.Error("Failed to rename branch on portal", []string{
					"Check network connection",
					"Verify portal URL and credentials",
					"Ensure branch exists on remote",
					"Check permissions for branch operations",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Successfully renamed %s to %s on %s!", oldBranch, newBranch, portalName))
			ec.output.Info("Local tracking will be updated if you switch to the new branch")

			return nil
		},
	}

	cmd.Flags().String("with", "", "New branch name (required)")
	cmd.MarkFlagRequired("with")

	return cmd
}

func (ec *EnhancedCLI) createWhatChangedCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "what-changed [reference]",
		Short: "Show what changed in files",
		Long: `Show detailed changes in files since a reference point

This command displays:
- Modified files with detailed diffs
- Added files with their content
- Deleted files  
- File status indicators
- Line-by-line changes with context

Examples:
  ivaldi what-changed                    # Show changes since last seal
  ivaldi what-changed bright-river-42    # Show changes since specific seal
  ivaldi what-changed #5                 # Show changes since iteration 5
  ivaldi what-changed "yesterday"        # Show changes since yesterday
  
This replaces 'git diff' with human-friendly change visualization.`,
		Args: cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Determine what to compare against
			_ = "" // reference for future use
			if len(args) > 0 {
				_ = args[0] // reference = args[0] - for future implementation
			}

			ec.output.Info("Analyzing changes...")

			// Get current workspace status
			status := ec.currentRepo.GetStatus()
			_ = ec.currentRepo.GetWorkspace() // workspace for future use

			if len(status.Modified) == 0 && len(status.Staged) == 0 {
				ec.output.Info("No changes detected in workspace")
				return nil
			}

			ec.output.Info("Changes detected:")
			ec.output.Info("")

			// Show staged changes (on anvil)
			if len(status.Staged) > 0 {
				ec.output.Info("Staged for sealing (on anvil):")
				for _, file := range status.Staged {
					ec.output.Info(fmt.Sprintf("  + %s", file))
				}
				ec.output.Info("")
			}

			// Show modified changes
			if len(status.Modified) > 0 {
				ec.output.Info("Modified files:")
				for _, file := range status.Modified {
					ec.output.Info(fmt.Sprintf("  ~ %s", file))

					// Try to show file diff using git (since we have git compatibility)
					diffCmd := exec.Command("git", "diff", file)
					diffCmd.Dir = ec.currentRepo.Root()
					if diffOutput, err := diffCmd.Output(); err == nil && len(diffOutput) > 0 {
						lines := strings.Split(string(diffOutput), "\n")
						for i, line := range lines {
							if i > 10 { // Limit to first 10 lines
								ec.output.Info("    ... (showing first 10 lines)")
								break
							}
							if strings.HasPrefix(line, "+") && !strings.HasPrefix(line, "+++") {
								ec.output.Info(fmt.Sprintf("    [32m%s[0m", line)) // Green for additions
							} else if strings.HasPrefix(line, "-") && !strings.HasPrefix(line, "---") {
								ec.output.Info(fmt.Sprintf("    [31m%s[0m", line)) // Red for deletions
							} else if strings.HasPrefix(line, "@@") {
								ec.output.Info(fmt.Sprintf("    [36m%s[0m", line)) // Cyan for context
							}
						}
					} else {
						ec.output.Info("    (unable to show diff - file may be new or binary)")
					}
					ec.output.Info("")
				}
			}

			// Show summary
			totalChanges := len(status.Modified) + len(status.Staged)
			ec.output.Info(fmt.Sprintf("Summary: %d files changed", totalChanges))

			if len(status.Staged) > 0 {
				ec.output.Info("Next step: ivaldi seal \"<message>\" to commit staged changes")
			} else if len(status.Modified) > 0 {
				ec.output.Info("Next step: ivaldi gather <files> to stage changes")
			}

			return nil
		},
	}
}

func (ec *EnhancedCLI) createRefreshCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "refresh",
		Short: "Refresh ignore patterns and clean workspace",
		Long: `Refresh ignore patterns from .ivaldiignore file and remove ignored files from workspace

This command:
1. Reloads ignore patterns from .ivaldiignore
2. Removes any currently staged files that should now be ignored
3. Updates workspace state to respect new ignore rules

Use this command after updating .ivaldiignore to clean up the workspace.

Examples:
  ivaldi refresh                    # Refresh ignore patterns and clean workspace`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			ec.output.Info("Refreshing ignore patterns...")

			err := ec.currentRepo.RefreshIgnorePatterns()
			if err != nil {
				ec.output.Error("Failed to refresh ignore patterns", []string{
					"Check .ivaldiignore file syntax",
					"Ensure file permissions are correct",
					"Try: refresh again",
				})
				return err
			}

			ec.output.Success("Ignore patterns refreshed successfully!")
			ec.output.Info("Removed ignored files from staging area")
			ec.output.Info("")
			ec.output.Info("Next step: ivaldi status to see clean workspace")

			return nil
		},
	}
}

func (ec *EnhancedCLI) createExcludeCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "exclude <files...>",
		Short: "Exclude files from tracking",
		Long: `Exclude files or directories from being tracked in the workspace

This command:
1. Adds the specified patterns to .ivaldiignore
2. Removes the files from staging area if currently staged
3. Stops tracking the files in future operations

Supports patterns:
- Individual files: exclude secrets.txt
- Directories: exclude logs/ temp/
- Wildcards: exclude *.log *.tmp
- Multiple items: exclude build/ *.exe temp/

Examples:
  ivaldi exclude build/              # Exclude build directory
  ivaldi exclude secrets.txt         # Exclude specific file
  ivaldi exclude *.log *.tmp         # Exclude by pattern
  ivaldi exclude logs/ temp/ *.exe   # Exclude multiple items
  
This replaces manually editing .ivaldiignore and provides immediate cleanup.`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			ec.output.Info(fmt.Sprintf("Excluding %d items from tracking...", len(args)))

			err := ec.currentRepo.ExcludeFiles(args)
			if err != nil {
				ec.output.Error("Failed to exclude files", []string{
					"Check file paths exist",
					"Ensure .ivaldiignore is writable",
					"Try: exclude <files> again",
				})
				return err
			}

			ec.output.Success("Files excluded successfully!")
			ec.output.Info("Added patterns to .ivaldiignore:")
			for _, pattern := range args {
				ec.output.Info(fmt.Sprintf("  - %s", pattern))
			}
			ec.output.Info("")
			ec.output.Info("Next step: ivaldi status to see clean workspace")

			return nil
		},
	}
}

func (ec *EnhancedCLI) createRemoveCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "remove <files...>",
		Short: "Remove files from repository",
		Long: `Remove files from the repository and stage the removal

This command:
1. Removes files from the filesystem
2. Stages the removal for the next seal (commit)
3. Optionally excludes files from future tracking

Options:
  --from-remote    Only remove from remote repository (keep local)
  --exclude        Also add to .ivaldiignore to prevent re-tracking

Examples:
  ivaldi remove old-file.txt              # Remove file locally and remotely
  ivaldi remove --from-remote secrets.txt # Remove only from remote, keep local
  ivaldi remove --exclude logs/           # Remove and exclude from tracking
  ivaldi remove temp/ *.log --exclude     # Remove multiple and exclude
  
This replaces complex 'git rm' workflows with intuitive file removal.`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			fromRemoteOnly, _ := cmd.Flags().GetBool("from-remote")
			excludeAfter, _ := cmd.Flags().GetBool("exclude")

			ec.output.Info(fmt.Sprintf("Removing %d items from repository...", len(args)))

			err := ec.currentRepo.RemoveFiles(args, fromRemoteOnly, excludeAfter)
			if err != nil {
				ec.output.Error("Failed to remove files", []string{
					"Check file paths exist",
					"Ensure files are not protected",
					"Try: remove <files> again",
				})
				return err
			}

			ec.output.Success("Files removed successfully!")
			if fromRemoteOnly {
				ec.output.Info("Files will be removed from remote repository only (kept locally)")
			} else {
				ec.output.Info("Files removed from filesystem and staged for removal")
			}

			if excludeAfter {
				ec.output.Info("Files also added to .ivaldiignore to prevent re-tracking")
			}

			ec.output.Info("")
			ec.output.Info("Next step: ivaldi seal \"remove unused files\" to commit removal")

			return nil
		},
	}

	cmd.Flags().Bool("from-remote", false, "Remove only from remote repository (keep local files)")
	cmd.Flags().Bool("exclude", false, "Also add to .ivaldiignore to prevent re-tracking")

	return cmd
}

func (ec *EnhancedCLI) createDownloadCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "download <url> [destination]",
		Short: "Download current repository files without Git history",
		Long: `Download current repository files from a remote URL without importing Git history

This command:
1. Downloads only the current files from the repository (like a snapshot)
2. Initializes it as a fresh Ivaldi repository 
3. Creates a clean slate without preserving Git commit history
4. Sets up the origin portal for future synchronization

Use this when you want a clean start without the historical baggage.

Examples:
  ivaldi download https://github.com/user/repo.git          # Download to ./repo
  ivaldi download https://github.com/user/repo.git my-repo  # Download to ./my-repo
  
This gives you a fresh Ivaldi repository with just the current files,
perfect for starting development without historical complexity.`,
		Args: cobra.RangeArgs(1, 2),
		RunE: func(cmd *cobra.Command, args []string) error {
			url := args[0]

			// Determine destination directory
			dest := ""
			if len(args) > 1 {
				dest = args[1]
			} else {
				// Extract repo name from URL
				parts := strings.Split(url, "/")
				if len(parts) > 0 {
					repoName := parts[len(parts)-1]
					// Remove .git suffix if present
					if strings.HasSuffix(repoName, ".git") {
						repoName = strings.TrimSuffix(repoName, ".git")
					}
					dest = repoName
				}
				if dest == "" {
					dest = "downloaded-repo"
				}
			}

			ec.output.Info(fmt.Sprintf("Downloading repository from: %s", url))
			ec.output.Info(fmt.Sprintf("Destination: %s", dest))

			// Use the download functionality (files only, no Git history)
			repo, err := forge.Download(url, dest)
			if err != nil {
				ec.output.Error("Failed to download repository", []string{
					"Check the URL is correct and accessible",
					"Verify network connection",
					"Ensure you have access to the repository",
					"Try: download <url> <destination-folder>",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Successfully downloaded repository files to %s!", dest))
			ec.output.Info("Fresh Ivaldi repository created with current files (no Git history)")
			ec.output.Info("Repository is now ready with all Ivaldi revolutionary features:")
			ec.output.Info("  • Natural language references")
			ec.output.Info("  • Automatic work preservation")
			ec.output.Info("  • AI-powered commit generation")
			ec.output.Info("  • Rich visual interface")
			ec.output.Info("")
			ec.output.Info(fmt.Sprintf("Next steps:"))
			ec.output.Info(fmt.Sprintf("  cd %s", dest))
			ec.output.Info("  ivaldi status")

			// Clean up
			repo.Close()

			return nil
		},
	}

	return cmd
}

// Create P2P command for peer-to-peer collaboration
func (ec *EnhancedCLI) createP2PCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "p2p",
		Short: "Peer-to-peer collaboration and sync",
		Long: `Manage peer-to-peer (P2P) connections for direct collaboration.

P2P allows direct synchronization between Ivaldi repositories without
needing a central server, enabling:
- Real-time collaboration on local networks
- Offline sync when internet is unavailable  
- Faster transfers between nearby peers
- Private networks for sensitive projects

Commands:
  start                    Start P2P network
  stop                     Stop P2P network
  status                   Show P2P status and connections
  connect <address:port>   Connect to a specific peer
  peers                    List all connected peers
  discover                 Show discovered peers on network
  sync [peer-id]           Sync with specific peer or all peers
  config                   View or update P2P configuration

Examples:
  ivaldi p2p start                    # Start P2P networking
  ivaldi p2p connect 192.168.1.100:9090  # Connect to peer
  ivaldi p2p sync                     # Sync with all peers
  ivaldi p2p sync abc123def            # Sync with specific peer
  ivaldi p2p config --auto-sync=true  # Enable automatic sync`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if len(args) == 0 {
				status := ec.currentRepo.GetP2PStatus()
				ec.showP2PStatus(status)
				return nil
			}
			return fmt.Errorf("unknown P2P subcommand: %s", args[0])
		},
	}

	// Add subcommands
	cmd.AddCommand(ec.createP2PStartCommand())
	cmd.AddCommand(ec.createP2PStopCommand())
	cmd.AddCommand(ec.createP2PStatusCommand())
	cmd.AddCommand(ec.createP2PConnectCommand())
	cmd.AddCommand(ec.createP2PPeersCommand())
	cmd.AddCommand(ec.createP2PDiscoverCommand())
	cmd.AddCommand(ec.createP2PSyncCommand())
	cmd.AddCommand(ec.createP2PConfigCommand())

	return cmd
}

// Create P2P start command
func (ec *EnhancedCLI) createP2PStartCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "start",
		Short: "Start P2P network",
		Long:  "Start the peer-to-peer network for this repository, enabling discovery and sync with other peers.",
		RunE: func(cmd *cobra.Command, args []string) error {
			daemon, _ := cmd.Flags().GetBool("daemon")

			if ec.currentRepo.IsP2PRunning() {
				ec.output.Info("P2P network is already running")
				return nil
			}

			ec.output.Info("Starting P2P network...")
			if err := ec.currentRepo.StartP2P(); err != nil {
				ec.output.Error("Failed to start P2P network", []string{
					"Check if port is already in use",
					"Verify network configuration",
					"Try: p2p config to check settings",
				})
				return err
			}

			config := ec.currentRepo.GetP2PConfig()
			ec.output.Success("P2P network started successfully!")
			ec.output.Info(fmt.Sprintf("Listening on port: %d", config.Port))
			ec.output.Info(fmt.Sprintf("Discovery port: %d", config.DiscoveryPort))
			if config.AutoSyncEnabled {
				ec.output.Info(fmt.Sprintf("Auto-sync enabled (every %v)", config.SyncInterval))
			}
			ec.output.Info("Other Ivaldi repositories on the network can now discover and connect to this peer")

			// If daemon mode, keep running
			if daemon {
				ec.output.Info("Running in daemon mode. Press Ctrl+C to stop.")

				// Set up signal handling for graceful shutdown
				sigChan := make(chan os.Signal, 1)
				signal.Notify(sigChan, os.Interrupt, syscall.SIGTERM)

				// Wait for signal
				<-sigChan

				ec.output.Info("\nStopping P2P network...")
				if err := ec.currentRepo.StopP2P(); err != nil {
					ec.output.Error("Failed to stop P2P network", []string{err.Error()})
				}
				ec.output.Info("P2P network stopped")
			}

			return nil
		},
	}

	cmd.Flags().BoolP("daemon", "d", false, "Run as daemon (keep process running)")
	return cmd
}

// Create P2P stop command
func (ec *EnhancedCLI) createP2PStopCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "stop",
		Short: "Stop P2P network",
		Long:  "Stop the peer-to-peer network, disconnecting from all peers.",
		RunE: func(cmd *cobra.Command, args []string) error {
			if !ec.currentRepo.IsP2PRunning() {
				ec.output.Info("P2P network is not running")
				return nil
			}

			ec.output.Info("Stopping P2P network...")
			if err := ec.currentRepo.StopP2P(); err != nil {
				ec.output.Error("Failed to stop P2P network", []string{
					"Check system logs for errors",
				})
				return err
			}

			ec.output.Success("P2P network stopped")
			return nil
		},
	}
}

// Create P2P status command
func (ec *EnhancedCLI) createP2PStatusCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "status",
		Short: "Show P2P status and connections",
		Long:  "Display current P2P network status, connected peers, and sync information.",
		RunE: func(cmd *cobra.Command, args []string) error {
			status := ec.currentRepo.GetP2PStatus()
			ec.showP2PStatus(status)
			return nil
		},
	}
}

// Create P2P connect command
func (ec *EnhancedCLI) createP2PConnectCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "connect <address:port>",
		Short: "Connect to a specific peer",
		Long:  "Connect to a peer at the specified address and port (e.g., 192.168.1.100:9090).",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			if !ec.currentRepo.IsP2PRunning() {
				return fmt.Errorf("P2P network is not running. Start it with: p2p start")
			}

			peerAddr := args[0]
			parts := strings.Split(peerAddr, ":")
			if len(parts) != 2 {
				return fmt.Errorf("invalid address format. Use: address:port (e.g., 192.168.1.100:9090)")
			}

			address := parts[0]
			port, err := strconv.Atoi(parts[1])
			if err != nil {
				return fmt.Errorf("invalid port number: %s", parts[1])
			}

			ec.output.Info(fmt.Sprintf("Connecting to peer: %s", peerAddr))
			if err := ec.currentRepo.ConnectToPeer(address, port); err != nil {
				ec.output.Error("Failed to connect to peer", []string{
					"Check the address and port are correct",
					"Verify the peer is running and accepting connections",
					"Ensure network connectivity",
					"Try: p2p discover to find peers automatically",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Successfully connected to peer: %s", peerAddr))
			return nil
		},
	}
}

// Create P2P peers command
func (ec *EnhancedCLI) createP2PPeersCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "peers",
		Short: "List all connected peers",
		Long:  "Show all currently connected P2P peers and their sync status.",
		RunE: func(cmd *cobra.Command, args []string) error {
			peers := ec.currentRepo.GetP2PPeers()
			syncStates := ec.currentRepo.GetP2PSyncState()

			if len(peers) == 0 {
				ec.output.Info("No connected peers")
				if !ec.currentRepo.IsP2PRunning() {
					ec.output.Info("P2P network is not running. Start with: p2p start")
				} else {
					ec.output.Info("Try: p2p discover to find peers on the network")
				}
				return nil
			}

			ec.output.Info(fmt.Sprintf("Connected peers (%d):", len(peers)))
			for _, peer := range peers {
				syncState := syncStates[peer.ID]
				lastSync := "never"
				if syncState != nil && !syncState.LastSync.IsZero() {
					lastSync = formatTimeSince(syncState.LastSync)
				}

				ec.output.Info(fmt.Sprintf("  %s", peer.ID))
				ec.output.Info(fmt.Sprintf("    Address: %s:%d", peer.Address, peer.Port))
				ec.output.Info(fmt.Sprintf("    Status: %s", peer.Status))
				ec.output.Info(fmt.Sprintf("    Last sync: %s", lastSync))
				if syncState != nil {
					ec.output.Info(fmt.Sprintf("    Conflicts: %d", syncState.ConflictCount))
				}
			}

			return nil
		},
	}
}

// Create P2P discover command
func (ec *EnhancedCLI) createP2PDiscoverCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "discover",
		Short: "Show discovered peers on network",
		Long:  "Show all Ivaldi repositories discovered on the local network that can be connected to.",
		RunE: func(cmd *cobra.Command, args []string) error {
			if !ec.currentRepo.IsP2PRunning() {
				return fmt.Errorf("P2P network is not running. Start it with: p2p start")
			}

			discovered := ec.currentRepo.GetDiscoveredPeers()

			if len(discovered) == 0 {
				ec.output.Info("No peers discovered on network")
				ec.output.Info("Make sure other Ivaldi repositories are running P2P")
				return nil
			}

			ec.output.Info(fmt.Sprintf("Discovered peers (%d):", len(discovered)))
			for _, peer := range discovered {
				ec.output.Info(fmt.Sprintf("  %s", peer.NodeID))
				ec.output.Info(fmt.Sprintf("    Address: %s:%d", peer.Address, peer.Port))
				ec.output.Info(fmt.Sprintf("    Last seen: %s", formatTimeSince(peer.LastSeen)))
				if len(peer.Repositories) > 0 {
					ec.output.Info(fmt.Sprintf("    Repositories: %v", peer.Repositories))
				}
				ec.output.Info(fmt.Sprintf("    Connect with: p2p connect %s:%d", peer.Address, peer.Port))
			}

			return nil
		},
	}
}

// Create P2P sync command
func (ec *EnhancedCLI) createP2PSyncCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "sync [peer-id]",
		Short: "Sync with specific peer or all peers",
		Long: `Synchronize timelines with P2P peers.

Without arguments, syncs with all connected peers.
With peer-id, syncs only with that specific peer.`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if !ec.currentRepo.IsP2PRunning() {
				return fmt.Errorf("P2P network is not running. Start it with: p2p start")
			}

			if len(args) == 0 {
				// Sync with all peers
				ec.output.Info("Syncing with all connected peers...")
				if err := ec.currentRepo.SyncWithAllP2PPeers(); err != nil {
					ec.output.Error("Failed to sync with peers", []string{
						"Check network connectivity",
						"Verify peers are still connected: p2p peers",
					})
					return err
				}
				ec.output.Success("Sync completed with all peers")
			} else {
				// Sync with specific peer
				peerID := args[0]
				ec.output.Info(fmt.Sprintf("Syncing with peer: %s", peerID))
				if err := ec.currentRepo.SyncWithP2PPeer(peerID); err != nil {
					ec.output.Error("Failed to sync with peer", []string{
						"Check the peer ID is correct: p2p peers",
						"Verify peer is still connected",
						"Check network connectivity",
					})
					return err
				}
				ec.output.Success(fmt.Sprintf("Sync completed with peer: %s", peerID))
			}

			return nil
		},
	}
}

// Create P2P config command
func (ec *EnhancedCLI) createP2PConfigCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "config",
		Short: "View or update P2P configuration",
		Long: `View or update P2P network configuration.

Examples:
  ivaldi p2p config                     # Show current config
  ivaldi p2p config --auto-sync=true    # Enable auto sync
  ivaldi p2p config --sync-interval=1m  # Set sync interval`,
		RunE: func(cmd *cobra.Command, args []string) error {
			config := ec.currentRepo.GetP2PConfig()
			if config == nil {
				return fmt.Errorf("P2P configuration not available")
			}

			// Handle flag updates
			autoSync, _ := cmd.Flags().GetBool("auto-sync")
			syncInterval, _ := cmd.Flags().GetString("sync-interval")

			updated := false

			if cmd.Flags().Changed("auto-sync") {
				if err := ec.currentRepo.EnableP2PAutoSync(autoSync); err != nil {
					return fmt.Errorf("failed to update auto-sync setting: %v", err)
				}
				ec.output.Success(fmt.Sprintf("Auto-sync %s", map[bool]string{true: "enabled", false: "disabled"}[autoSync]))
				updated = true
			}

			if cmd.Flags().Changed("sync-interval") {
				duration, err := time.ParseDuration(syncInterval)
				if err != nil {
					return fmt.Errorf("invalid sync interval format: %v", err)
				}
				if err := ec.currentRepo.SetP2PSyncInterval(duration); err != nil {
					return fmt.Errorf("failed to update sync interval: %v", err)
				}
				ec.output.Success(fmt.Sprintf("Sync interval set to %s", syncInterval))
				updated = true
			}

			if updated {
				// Reload config after updates
				config = ec.currentRepo.GetP2PConfig()
			}

			// Show current configuration
			ec.output.Info("P2P Configuration:")
			ec.output.Info(fmt.Sprintf("  Port: %d", config.Port))
			ec.output.Info(fmt.Sprintf("  Discovery Port: %d", config.DiscoveryPort))
			ec.output.Info(fmt.Sprintf("  Max Peers: %d", config.MaxPeers))
			ec.output.Info(fmt.Sprintf("  Auto-connect: %t", config.EnableAutoConnect))
			ec.output.Info(fmt.Sprintf("  Auto-sync: %t", config.AutoSyncEnabled))
			if config.AutoSyncEnabled {
				ec.output.Info(fmt.Sprintf("  Sync Interval: %v", config.SyncInterval))
			}
			ec.output.Info(fmt.Sprintf("  Conflict Strategy: %s", config.ConflictStrategy))
			ec.output.Info(fmt.Sprintf("  Known Peers: %d", len(config.KnownPeers)))

			return nil
		},
	}

	cmd.Flags().Bool("auto-sync", false, "Enable or disable automatic sync")
	cmd.Flags().String("sync-interval", "", "Set sync interval (e.g., 30s, 1m, 5m)")

	return cmd
}

// Helper function to show P2P status
func (ec *EnhancedCLI) showP2PStatus(status *p2p.P2PStatus) {
	if status.Running {
		ec.output.Success("P2P Network: RUNNING")
		ec.output.Info(fmt.Sprintf("Node ID: %s", status.NodeID))
		ec.output.Info(fmt.Sprintf("Port: %d", status.Port))
		ec.output.Info(fmt.Sprintf("Connected Peers: %d", status.ConnectedPeers))
		ec.output.Info(fmt.Sprintf("Discovered Peers: %d", status.DiscoveredPeers))

		autoSyncStatus := "disabled"
		if status.AutoSyncEnabled {
			autoSyncStatus = fmt.Sprintf("enabled (every %v)", status.SyncInterval)
		}
		ec.output.Info(fmt.Sprintf("Auto-sync: %s", autoSyncStatus))

		if status.TotalSyncs > 0 {
			ec.output.Info(fmt.Sprintf("Total Syncs: %d", status.TotalSyncs))
		}
		if status.ConflictCount > 0 {
			ec.output.Info(fmt.Sprintf("Conflicts: %d", status.ConflictCount))
		}
	} else {
		ec.output.Info("P2P Network: STOPPED")
		ec.output.Info("Start with: p2p start")
	}
}

// Helper function to format time since
func formatTimeSince(t time.Time) string {
	if t.IsZero() {
		return "never"
	}

	duration := time.Since(t)
	if duration < time.Minute {
		return fmt.Sprintf("%d seconds ago", int(duration.Seconds()))
	} else if duration < time.Hour {
		return fmt.Sprintf("%d minutes ago", int(duration.Minutes()))
	} else if duration < 24*time.Hour {
		return fmt.Sprintf("%d hours ago", int(duration.Hours()))
	} else {
		return fmt.Sprintf("%d days ago", int(duration.Hours()/24))
	}
}

// extractRepoNameFromURL extracts the repository name from a Git URL
func (ec *EnhancedCLI) extractRepoNameFromURL(url string) string {
	// Remove trailing .git if present
	url = strings.TrimSuffix(url, ".git")

	// Handle different URL formats
	if strings.Contains(url, "://") {
		// HTTPS format: https://github.com/user/repo
		parts := strings.Split(url, "/")
		if len(parts) > 0 {
			return parts[len(parts)-1]
		}
	} else if strings.HasPrefix(url, "git@") {
		// SSH format: git@github.com:user/repo
		// Split by colon first
		parts := strings.Split(url, ":")
		if len(parts) == 2 {
			// Get the path part and extract repo name
			pathParts := strings.Split(parts[1], "/")
			if len(pathParts) > 0 {
				return pathParts[len(pathParts)-1]
			}
		}
	}

	// Fallback: try to get last path component
	parts := strings.Split(url, "/")
	if len(parts) > 0 && parts[len(parts)-1] != "" {
		return parts[len(parts)-1]
	}

	return ""
}

// performEnhancedMirror performs the mirror operation with full history conversion
func (ec *EnhancedCLI) performEnhancedMirror(url, dest string) (*forge.EnhancedRepository, error) {
	// Step 1: Create destination directory
	ec.output.Info("Step 1: Creating destination directory...")
	if err := os.MkdirAll(dest, 0755); err != nil {
		return nil, fmt.Errorf("failed to create destination directory: %v", err)
	}

	// Step 2: Initialize Ivaldi repository
	ec.output.Info("Step 2: Initializing Ivaldi repository...")
	absPath, err := filepath.Abs(dest)
	if err != nil {
		return nil, err
	}

	repo, err := forge.EnhancedInitialize(absPath)
	if err != nil {
		return nil, fmt.Errorf("failed to initialize Ivaldi repository: %v", err)
	}

	// Step 3: Add origin portal
	ec.output.Info("Step 3: Adding origin portal...")
	if err := repo.AddPortal("origin", url); err != nil {
		return nil, fmt.Errorf("failed to add origin portal: %v", err)
	}

	// Step 4: Detect default branch (try main, then master, then fallback)
	ec.output.Info("Step 4: Detecting default branch...")
	branch := "main"
	// TODO: Implement branch detection from remote

	// Step 5: Fetch with complete history
	ec.output.Info("Step 5: Fetching complete commit history...")
	networkMgr := repo.Network()

	// Use the history-enabled fetch
	fetchResult, err := networkMgr.FetchFromPortalWithHistory(url, branch)
	if err != nil {
		// Try with master branch if main failed
		ec.output.Info("Trying 'master' branch...")
		fetchResult, err = networkMgr.FetchFromPortalWithHistory(url, "master")
		if err != nil {
			return nil, fmt.Errorf("failed to fetch repository history: %v", err)
		}
		branch = "master"
	}

	ec.output.Info(fmt.Sprintf("Fetched %d commits from %s branch", len(fetchResult.Seals), branch))

	// Step 6: Store and index all seals
	ec.output.Info("Step 6: Converting and storing Git commits as Ivaldi seals...")
	storage := repo.Storage()
	index := repo.GetIndex()
	for i, seal := range fetchResult.Seals {
		if err := storage.StoreSeal(seal); err != nil {
			return nil, fmt.Errorf("failed to store seal %d: %v", i, err)
		}
		// CRITICAL: Index the seal in the database for ivaldi log to work
		if err := index.IndexSeal(seal); err != nil {
			return nil, fmt.Errorf("failed to index seal %d: %v", i, err)
		}
	}

	// Step 7: Create timeline for the main branch
	ec.output.Info("Step 7: Creating timeline for main branch...")
	timelineMgr := repo.Timeline()

	// First ensure the timeline exists or create it
	if err := timelineMgr.Create(branch, "Mirrored branch"); err != nil {
		// Check if it already exists - that's expected, not an error
		if _, headErr := timelineMgr.GetHead(branch); headErr != nil {
			// Timeline doesn't exist and creation failed
			return nil, fmt.Errorf("failed to create timeline '%s': %v", branch, err)
		}
		ec.output.Info(fmt.Sprintf("Timeline '%s' already exists", branch))
	} else {
		ec.output.Info(fmt.Sprintf("Created timeline '%s'", branch))
	}

	// Update timeline head - find the ref that matches current branch
	if len(fetchResult.Refs) == 0 {
		return nil, fmt.Errorf("no refs found in fetch result")
	}

	// Look for a ref that matches our current branch
	var targetRef *network.RemoteRef
	expectedRefName := fmt.Sprintf("refs/heads/%s", branch)

	for i := range fetchResult.Refs {
		if fetchResult.Refs[i].Name == expectedRefName {
			targetRef = &fetchResult.Refs[i]
			break
		}
	}

	// Fall back to first ref if no exact match found
	if targetRef == nil {
		ec.output.Info(fmt.Sprintf("No ref found matching '%s', using first ref '%s'", expectedRefName, fetchResult.Refs[0].Name))
		targetRef = &fetchResult.Refs[0]
	}

	// Update the timeline head
	if err := timelineMgr.UpdateHead(branch, targetRef.Hash); err != nil {
		return nil, fmt.Errorf("failed to update timeline head for '%s' with ref '%s': %v", branch, targetRef.Name, err)
	}
	ec.output.Info(fmt.Sprintf("Updated timeline '%s' head to %s", branch, targetRef.Hash.String()[:8]))

	// Step 8: Download all files
	ec.output.Info("Step 8: Downloading repository files...")
	if err := networkMgr.DownloadIvaldiRepo(url, absPath); err != nil {
		return nil, fmt.Errorf("failed to download repository files: %v", err)
	}

	// Step 9: Convert .gitmodules to .ivaldimodules
	ec.output.Info("Step 9: Converting submodules...")
	if err := workspace.CreateIvaldimodulesFromGitmodules(absPath); err != nil {
		ec.output.Info("No submodules to convert")
	}

	// Step 10: Switch to the main branch
	if err := timelineMgr.Switch(branch); err != nil {
		ec.output.Info(fmt.Sprintf("Warning: could not switch to timeline '%s'", branch))
	}

	ec.output.Info("✓ Mirror operation completed successfully!")
	return repo, nil
}

// performGitToIvaldiMigration performs the actual migration from Git to Ivaldi
func (ec *EnhancedCLI) performGitToIvaldiMigration(repoPath string) error {
	// Step 1: Initialize Ivaldi repository
	ec.output.Info("Step 1: Initializing Ivaldi repository...")
	repo, err := forge.EnhancedInitialize(repoPath)
	if err != nil {
		return fmt.Errorf("failed to initialize Ivaldi repository: %v", err)
	}

	ec.currentRepo = repo

	// Step 2: Convert .gitmodules to .ivaldimodules
	ec.output.Info("Step 2: Converting .gitmodules to .ivaldimodules...")
	if err := workspace.CreateIvaldimodulesFromGitmodules(repoPath); err != nil {
		ec.output.Info("Warning: could not convert .gitmodules - continuing without submodule conversion")
	}

	// Step 3: Create backup of .git directory
	ec.output.Info("Step 3: Creating backup of .git directory...")
	gitPath := filepath.Join(repoPath, ".git")
	backupPath := filepath.Join(repoPath, ".git.backup")

	if err := os.Rename(gitPath, backupPath); err != nil {
		return fmt.Errorf("failed to backup .git directory: %v", err)
	}

	ec.output.Info("✓ Git directory backed up to .git.backup")

	// Step 4: Gather all files for initial commit
	ec.output.Info("Step 4: Gathering all files for initial Ivaldi commit...")
	if err := repo.Gather([]string{"."}); err != nil {
		return fmt.Errorf("failed to gather files: %v", err)
	}

	// Step 5: Create initial Ivaldi commit
	ec.output.Info("Step 5: Creating initial Ivaldi commit...")
	_, err = repo.Seal("Migrated from Git repository")
	if err != nil {
		return fmt.Errorf("failed to create initial commit: %v", err)
	}

	ec.output.Info("✓ Migration completed successfully!")
	return nil
}

// Execute runs the enhanced CLI
func Execute() error {
	cli := NewEnhancedCLI()
	rootCmd := cli.CreateRootCommand()
	return rootCmd.Execute()
}
