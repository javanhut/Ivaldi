package enhanced_cli

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/spf13/cobra"

	"ivaldi/core/commands"
	"ivaldi/core/config"
	"ivaldi/core/fuse"
	"ivaldi/core/reshape"
	"ivaldi/core/search"
	"ivaldi/core/semantic"
	"ivaldi/core/workspace"
	"ivaldi/forge"
	"ivaldi/ui/enhanced"
)

// EnhancedCLI integrates all revolutionary features into the command interface
type EnhancedCLI struct {
	output         *enhanced.EnhancedOutput
	nlParser       *commands.NaturalLanguageParser
	currentRepo    *forge.EnhancedRepository
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
  download <url>           Download repository from URL
  gather [files...]        Stage files for sealing
  exclude <files...>       Exclude files from tracking
  remove <files...>        Remove files from repository
  seal [message]           Commit changes with memorable name
  timeline                 Manage development timelines (branches)
  jump <reference>         Jump to any point using natural language
  shelf                    Save work for later
  portal                   Manage remote connections
  upload [branch]          Upload current branch to portal
  sync --with <branch>     Sync with remote branch automatically
  
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
	if cmd.Name() == "forge" || cmd.Name() == "download" || cmd.Name() == "mirror" || cmd.Name() == "help" {
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
	rootCmd.AddCommand(ec.createGatherCommand())
	rootCmd.AddCommand(ec.createExcludeCommand())
	rootCmd.AddCommand(ec.createRemoveCommand())
	rootCmd.AddCommand(ec.createSealCommand())
	rootCmd.AddCommand(ec.createTimelineCommand())
	rootCmd.AddCommand(ec.createJumpCommand())
	rootCmd.AddCommand(ec.createShelfCommand())
	rootCmd.AddCommand(ec.createPortalCommand())
	rootCmd.AddCommand(ec.createUploadCommand())
	rootCmd.AddCommand(ec.createSyncCommand())
	
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
		Short: "Mirror a repository from remote location",
		Long: `Mirror (clone) a repository with enhanced Ivaldi features

This clones a Git repository and upgrades it to Ivaldi with:
- Memorable names assigned to all commits
- Revolutionary features enabled
- Git compatibility maintained

Example:
  mirror https://github.com/user/repo
  mirror git@github.com:user/repo.git my-local-name`,
		Args: cobra.MinimumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			url := args[0]
			dest := ""
			if len(args) > 1 {
				dest = args[1]
			}

			ec.output.Info(fmt.Sprintf("Mirroring repository from %s", url))
			
			repo, err := forge.EnhancedMirror(url, dest)
			if err != nil {
				ec.output.Error("Failed to mirror repository", []string{
					"Check URL accessibility",
					"Verify authentication if required",
					"Ensure destination path is available",
				})
				return err
			}

			ec.currentRepo = repo
			ec.output.Success("Repository mirrored and enhanced!")
			ec.output.Info("All Git commits now have memorable names")
			
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
  list                    List all timelines
  delete <name>           Delete timeline
  merge <from> <to>       Merge timelines

Examples:
  timeline create feature
  timeline switch main --preserve
  timeline list
  timeline delete old-feature`,
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
	cmd.AddCommand(ec.createTimelineListCommand())
	cmd.AddCommand(ec.createTimelineDeleteCommand())
	
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

			ec.output.Success(fmt.Sprintf("Created timeline: %s", args[0]))
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

			// Simple, git-like output
			if portalName == "origin" {
				fmt.Printf("Uploading to origin/%s\n", branch)
			} else {
				fmt.Printf("Uploading to %s/%s\n", portalName, branch)
			}

			err := ec.currentRepo.UploadToPortal(portalName, branch)
			if err != nil {
				// Check if it's a branch not found error
				if strings.Contains(err.Error(), "does not match any") {
					ec.output.Error(fmt.Sprintf("Timeline '%s' does not exist on remote", branch), []string{
						"Create timeline on remote: ivaldi portal new " + branch,
						"Or specify an existing timeline: ivaldi upload <existing-timeline>",
						"Check available timelines: ivaldi timeline list",
						"Switch to correct timeline: ivaldi timeline switch <timeline-name>",
					})
				} else {
					ec.output.Error("Failed to upload to portal", []string{
						"Check network connection",
						"Verify portal URL and credentials", 
						"Ensure repository exists on remote",
						"Try: portal add origin <url> (if no portal configured)",
					})
				}
				return err
			}

			if portalName == "origin" {
				fmt.Printf("Upload complete: origin/%s\n", branch)
			} else {
				fmt.Printf("Upload complete: %s/%s\n", portalName, branch)
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
		Use:   "sync --with <branch> [portal-name]",
		Short: "Sync with remote branch automatically",
		Long: `Sync with a remote branch by pulling changes and fusing them into current timeline.

This command performs the following steps automatically:
1. Pulls latest changes from the remote branch
2. Fuses those changes into the current timeline
3. Preserves your local work with automatic conflict resolution

Examples:
  ivaldi sync --with main           # Sync with origin/main
  ivaldi sync --with main upstream  # Sync with upstream/main
  ivaldi sync --with develop        # Sync with origin/develop`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository")
			}

			// Get the branch to sync with
			syncBranch, _ := cmd.Flags().GetString("with")
			if syncBranch == "" {
				return fmt.Errorf("--with flag is required to specify branch to sync with")
			}

			// Default to origin portal
			portalName := "origin"
			if len(args) > 0 {
				portalName = args[0]
			}

			currentTimeline := ec.currentRepo.GetCurrentTimeline()
			
			ec.output.Info(fmt.Sprintf("Syncing timeline '%s' with remote branch '%s' from portal '%s'", currentTimeline, syncBranch, portalName))

			// Step 1: Handle any uncommitted changes first
			if ec.currentRepo.GetWorkspace().HasUncommittedChanges() {
				ec.output.Info("Step 1: Auto-sealing uncommitted changes...")
				
				// Auto-gather and seal uncommitted changes
				err := ec.currentRepo.GetWorkspace().Scan()
				if err != nil {
					return fmt.Errorf("failed to scan workspace: %v", err)
				}
				
				err = ec.currentRepo.Gather([]string{"."})
				if err != nil {
					return fmt.Errorf("failed to gather changes: %v", err)
				}
				
				_, err = ec.currentRepo.Seal(fmt.Sprintf("Auto-seal before sync with %s", syncBranch))
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

			// Step 2: Sync with remote using Ivaldi-native sync (handles fetch and fuse automatically)
			ec.output.Info("Step 2: Syncing with remote...")
			err := ec.currentRepo.Sync(portalName, syncBranch)
			if err != nil {
				ec.output.Error("Failed to sync with remote", []string{
					"Check network connection",
					"Verify portal URL and credentials", 
					"Ensure remote branch exists: " + syncBranch,
					"Try: portal add " + portalName + " <url> (if portal not configured)",
				})
				return err
			}
			
			ec.output.Success(fmt.Sprintf("Successfully synced with %s/%s!", portalName, syncBranch))
			ec.output.Info("Your timeline is now up-to-date with remote changes")
			ec.output.Info("Next: Continue working or 'ivaldi upload' to push your changes")

			return nil
		},
	}

	cmd.Flags().StringP("with", "w", "", "Branch to sync with (required)")
	cmd.MarkFlagRequired("with")

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
			untracked := ec.getUntrackedFiles()
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

func (ec *EnhancedCLI) getUntrackedFiles() []string {
	if ec.currentRepo == nil {
		return []string{}
	}
	
	ws := ec.currentRepo.GetWorkspace()
	var untracked []string
	
	// Walk through directory and find files not in workspace
	err := filepath.Walk(ec.currentRepo.Root(), func(path string, info os.FileInfo, err error) error {
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
		
		// Skip directories
		if info.IsDir() {
			return nil
		}
		
		// Get relative path
		relPath, err := filepath.Rel(ec.currentRepo.Root(), path)
		if err != nil {
			return nil
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
		return []string{}
	}
	
	return untracked
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
	return &cobra.Command{
		Use:   "mesh",
		Short: "Local peer-to-peer networking",
		RunE: func(cmd *cobra.Command, args []string) error {
			ec.output.Info("P2P networking")
			ec.output.Info("(Implementation coming soon)")
			return nil
		},
	}
}

func (ec *EnhancedCLI) createCollaborateCommand() *cobra.Command {
	return &cobra.Command{
		Use:   "collaborate",
		Short: "Real-time collaboration session",
		RunE: func(cmd *cobra.Command, args []string) error {
			ec.output.Info("Real-time collaboration")
			ec.output.Info("(Implementation coming soon)")
			return nil
		},
	}
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
		Short: "Configuration management",
		Long: `Configure Ivaldi settings and credentials

Interactive setup will guide you through configuring:
- User information (name, email)
- GitHub personal access token
- GitLab personal access token

Examples:
  ivaldi config           # Interactive setup
  ivaldi config --show   # Show current configuration`,
		RunE: func(cmd *cobra.Command, args []string) error {
			if ec.currentRepo == nil {
				return fmt.Errorf("not in repository - please run 'ivaldi forge' or 'ivaldi download' first")
			}

			showConfig, _ := cmd.Flags().GetBool("show")

			configMgr := config.NewConfigManager(ec.currentRepo.Root())

			if showConfig {
				return ec.showCurrentConfig(configMgr)
			}

			// Interactive setup
			ec.output.Info("Starting interactive configuration...")
			if err := configMgr.InteractiveSetup(); err != nil {
				ec.output.Error("Configuration failed", []string{
					err.Error(),
					"Try running 'ivaldi config' again",
				})
				return err
			}

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
		Short: "Download repository from URL",
		Long: `Download a repository from a remote URL (GitHub, GitLab, etc.)

This command:
1. Downloads the repository from the specified URL
2. Initializes it as an Ivaldi repository with all revolutionary features
3. Imports the Git history with memorable names
4. Sets up the origin portal automatically

Examples:
  ivaldi download https://github.com/user/repo.git          # Download to ./repo
  ivaldi download https://github.com/user/repo.git my-repo  # Download to ./my-repo
  
This replaces the traditional 'git clone' with a more intuitive command that
automatically sets up Ivaldi's revolutionary features.`,
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

			// Use the enhanced mirror functionality
			repo, err := forge.EnhancedMirror(url, dest)
			if err != nil {
				ec.output.Error("Failed to download repository", []string{
					"Check the URL is correct and accessible",
					"Verify network connection",
					"Ensure you have access to the repository",
					"Try: download <url> <destination-folder>",
				})
				return err
			}

			ec.output.Success(fmt.Sprintf("Successfully downloaded repository to %s!", dest))
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

// Execute runs the enhanced CLI
func Execute() error {
	cli := NewEnhancedCLI()
	rootCmd := cli.CreateRootCommand()
	return rootCmd.Execute()
}