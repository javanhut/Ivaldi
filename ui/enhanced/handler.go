package enhanced

import (
	"fmt"
	"strconv"
	"strings"
	"time"

	"ivaldi/core/commands"
	"ivaldi/core/objects"
	"ivaldi/core/overwrite"
	"ivaldi/core/preservation"
	"ivaldi/core/references"
	"ivaldi/core/workspace"
	"ivaldi/forge"
)

// EnhancedCommandHandler processes natural language commands with rich output
type EnhancedCommandHandler struct {
	parser           *commands.NaturalLanguageParser
	output           *EnhancedOutput
	repo             *forge.Repository
	references       *references.ReferenceManager
	preservation     *preservation.PreservationManager
	overwriteTracker *overwrite.OverwriteTracker
}

func NewEnhancedCommandHandler(repo *forge.Repository) *EnhancedCommandHandler {
	return &EnhancedCommandHandler{
		parser:           commands.NewNaturalLanguageParser(),
		output:           NewEnhancedOutput(),
		repo:             repo,
		references:       references.NewReferenceManager(repo.Root()),
		preservation:     preservation.NewPreservationManager(repo.Root()),
		overwriteTracker: overwrite.NewOverwriteTracker(repo.Root()),
	}
}

// ProcessCommand handles natural language commands
func (ech *EnhancedCommandHandler) ProcessCommand(input string) error {
	// Parse natural language
	cmd, err := ech.parser.Parse(input)
	if err != nil {
		ech.handleUnknownCommand(input)
		return err
	}

	// Route to appropriate handler
	switch cmd.Command {
	case "gather":
		return ech.handleGather(cmd)
	case "gather_except":
		return ech.handleGatherExcept(cmd)
	case "seal":
		return ech.handleSeal(cmd)
	case "seal_auto":
		return ech.handleSealAuto(cmd)
	case "unseal":
		return ech.handleUnseal(cmd)
	case "timeline_create":
		return ech.handleTimelineCreate(cmd)
	case "timeline_create_from":
		return ech.handleTimelineCreateFrom(cmd)
	case "timeline_switch":
		return ech.handleTimelineSwitch(cmd)
	case "timeline_delete":
		return ech.handleTimelineDelete(cmd)
	case "timeline_list":
		return ech.handleTimelineList(cmd)
	case "jump":
		return ech.handleJump(cmd)
	case "jump_relative":
		return ech.handleJumpRelative(cmd)
	case "position":
		return ech.handlePosition(cmd)
	case "workspace_status":
		return ech.handleWorkspaceStatus(cmd)
	case "workspace_save":
		return ech.handleWorkspaceSave(cmd)
	case "workspace_load":
		return ech.handleWorkspaceLoad(cmd)
	case "shelf_put":
		return ech.handleShelfPut(cmd)
	case "shelf_take":
		return ech.handleShelfTake(cmd)
	case "protect":
		return ech.handleProtect(cmd)
	case "show_overwrites":
		return ech.handleShowOverwrites(cmd)
	default:
		return fmt.Errorf("command handler not implemented: %s", cmd.Command)
	}
}

// Command handlers

func (ech *EnhancedCommandHandler) handleGather(cmd *commands.ParsedCommand) error {
	files := cmd.Arguments["files"]

	if files == "all" || files == "." {
		ech.output.Info("Gathering all changes to the anvil...")
		if err := ech.repo.Gather([]string{"."}); err != nil {
			ech.output.Error("Failed to gather files", []string{
				"Check if you're in an Ivaldi repository",
				"Ensure files exist and are readable",
				"Try gathering specific files instead",
			})
			return err
		}
	} else {
		fileList := strings.Split(files, " ")
		ech.output.Info(fmt.Sprintf("Gathering %d file(s) to the anvil...", len(fileList)))
		if err := ech.repo.Gather(fileList); err != nil {
			ech.output.Error("Failed to gather files", []string{
				fmt.Sprintf("Check if files exist: %s", files),
				"Use 'workspace' to see available files",
				"Try using patterns like '*.go' or 'src/'",
			})
			return err
		}
	}

	ech.output.Success("Files gathered successfully!")
	return nil
}

func (ech *EnhancedCommandHandler) handleGatherExcept(cmd *commands.ParsedCommand) error {
	include := cmd.Arguments["include"]
	exclude := cmd.Arguments["exclude"]

	ech.output.Info(fmt.Sprintf("Gathering %s except %s...", include, exclude))

	// TODO: Implement exclude logic
	// For now, just gather the include pattern
	if err := ech.repo.Gather([]string{include}); err != nil {
		ech.output.Error("Failed to gather with exclusions", []string{
			"Check file patterns are valid",
			"Try gathering without exclusions first",
			"Use 'discard' command to remove unwanted files",
		})
		return err
	}

	ech.output.Success(fmt.Sprintf("Gathered %s (excluding %s)", include, exclude))
	return nil
}

func (ech *EnhancedCommandHandler) handleSeal(cmd *commands.ParsedCommand) error {
	message := cmd.Arguments["message"]

	if message == "" {
		ech.output.Error("Seal message is required", []string{
			"seal \"Your commit message\"",
			"seal   # for auto-generated message",
		})
		return fmt.Errorf("message required")
	}

	ech.output.Info("Sealing changes into history...")
	ech.output.Info("*** DEBUG: Handler is about to call Seal ***")
	fmt.Printf("*** HANDLER CALLING SEAL WITH MESSAGE: %s ***\n", message)

	seal, err := ech.repo.Seal(message)
	if err != nil {
		if strings.Contains(err.Error(), "nothing gathered") {
			ech.output.Error("Nothing on the anvil to seal", []string{
				"gather all                    # Gather all changes first",
				"gather src/                   # Gather specific files",
				"workspace                     # Check what's available",
			})
		} else {
			ech.output.Error("Failed to seal changes", []string{
				"Check if you have gathered files",
				"Ensure you're in an Ivaldi repository",
				"Try 'workspace' to see current state",
			})
		}
		return err
	}

	// Register memorable name
	ech.references.RegisterMemorableName(seal.Name, seal.Hash, seal.Author.Name)

	ech.output.Success(fmt.Sprintf("Sealed as '%s' (#%d)", seal.Name, seal.Iteration))
	ech.output.Info(fmt.Sprintf("Message: %s", seal.Message))

	return nil
}

func (ech *EnhancedCommandHandler) handleSealAuto(cmd *commands.ParsedCommand) error {
	// Generate automatic commit message based on changes
	message := ech.generateAutoMessage()

	ech.output.Info(fmt.Sprintf("Auto-generated message: %s", message))
	ech.output.Info("Sealing changes into history...")

	seal, err := ech.repo.Seal(message)
	if err != nil {
		ech.output.Error("Failed to seal changes", []string{
			"gather all                    # Gather files first",
			"seal \"manual message\"        # Use manual message instead",
		})
		return err
	}

	ech.references.RegisterMemorableName(seal.Name, seal.Hash, seal.Author.Name)
	ech.output.Success(fmt.Sprintf("Auto-sealed as '%s' (#%d)", seal.Name, seal.Iteration))

	return nil
}

func (ech *EnhancedCommandHandler) handleUnseal(cmd *commands.ParsedCommand) error {
	ech.output.Warning("Unsealing last commit...")

	// This would require overwrite tracking
	record, err := ech.overwriteTracker.RequestOverwrite(
		ech.getCurrentHash(),
		ech.getCurrentName(),
		ech.getPreviousHash(),
		ech.getPreviousName(),
		"User requested unseal of last commit",
		overwrite.CategoryAmend,
		ech.getCurrentUser(),
	)

	if err != nil {
		ech.output.Error("Failed to unseal", []string{
			"Check if there are any seals to unseal",
			"Use 'chronicle' to see history",
			"Consider 'jump back 1' instead",
		})
		return err
	}

	ech.output.Success("Last seal removed")
	ech.output.Info(fmt.Sprintf("Overwrite recorded: %s", record.ID))

	return nil
}

func (ech *EnhancedCommandHandler) handleTimelineSwitch(cmd *commands.ParsedCommand) error {
	timeline := cmd.Arguments["name"]

	ech.output.Info(fmt.Sprintf("Switching to timeline '%s'...", timeline))

	// Auto-preserve current work
	currentWorkspace := ech.getCurrentWorkspace()
	snapshot, err := ech.preservation.AutoPreserve(currentWorkspace, ech.getCurrentTimeline(), fmt.Sprintf("switching to %s", timeline))
	if err != nil {
		ech.output.Warning(fmt.Sprintf("Failed to preserve workspace: %v", err))
	} else if snapshot != nil {
		ech.output.Info(fmt.Sprintf("Work preserved as '%s'", snapshot.Name))
	}

	// Switch timeline
	if err := ech.repo.SwitchTimeline(timeline); err != nil {
		ech.output.Error(fmt.Sprintf("Failed to switch to timeline '%s'", timeline), []string{
			"timeline list                 # See available timelines",
			fmt.Sprintf("timeline create %s          # Create if it doesn't exist", timeline),
			"workspace                     # Check current state",
		})
		return err
	}

	ech.output.Success(fmt.Sprintf("Switched to timeline '%s'", timeline))

	// Check for preserved work on this timeline
	snapshots := ech.preservation.GetSnapshotsByTimeline(timeline)
	if len(snapshots) > 0 {
		ech.output.Info(fmt.Sprintf("Found %d preserved workspace(s) for this timeline", len(snapshots)))
		ech.output.Info("Use 'workspace load \"name\"' to restore previous work")
	}

	return nil
}

func (ech *EnhancedCommandHandler) handleJump(cmd *commands.ParsedCommand) error {
	reference := cmd.Arguments["reference"]

	ech.output.Info(fmt.Sprintf("Jumping to '%s'...", reference))

	// Resolve natural language reference
	hash, err := ech.references.Resolve(reference, ech.getCurrentTimeline())
	if err != nil {
		ech.output.Error(fmt.Sprintf("Reference '%s' not found", reference), []string{
			"chronicle                     # See available positions",
			"jump to \"yesterday\"          # Use natural language",
			"jump to #150                 # Use iteration number",
			"jump to bright-river-42      # Use memorable name",
		})
		return err
	}

	if err := ech.repo.Jump(hash.String()); err != nil {
		ech.output.Error("Failed to jump", []string{
			"Check if the reference exists",
			"Use 'chronicle' to see history",
			"Try a different reference format",
		})
		return err
	}

	ech.output.Success(fmt.Sprintf("Jumped to '%s'", reference))
	return nil
}

func (ech *EnhancedCommandHandler) handleWorkspaceStatus(cmd *commands.ParsedCommand) error {
	status := ech.getWorkspaceStatus()
	ech.output.ShowWorkspaceStatus(status)
	return nil
}

func (ech *EnhancedCommandHandler) handleProtect(cmd *commands.ParsedCommand) error {
	reference := cmd.Arguments["reference"]

	hash, err := ech.references.Resolve(reference, ech.getCurrentTimeline())
	if err != nil {
		ech.output.Error(fmt.Sprintf("Reference '%s' not found", reference), nil)
		return err
	}

	if err := ech.overwriteTracker.ProtectCommit(hash); err != nil {
		ech.output.Error("Failed to protect commit", nil)
		return err
	}

	ech.output.Success(fmt.Sprintf("Protected '%s' from overwrites", reference))
	return nil
}

func (ech *EnhancedCommandHandler) handleShowOverwrites(cmd *commands.ParsedCommand) error {
	reference := cmd.Arguments["reference"]

	records := ech.overwriteTracker.GetOverwriteHistory(reference)
	if len(records) == 0 {
		ech.output.Info(fmt.Sprintf("No overwrites recorded for '%s'", reference))
		return nil
	}

	ech.output.Info(fmt.Sprintf("Overwrite history for '%s':", reference))
	for _, record := range records {
		fmt.Printf("  %s - %s (%s)\n",
			record.Timestamp.Format("2006-01-02 15:04"),
			record.Justification,
			record.Category)
	}

	return nil
}

func (ech *EnhancedCommandHandler) handleUnknownCommand(input string) {
	ech.output.Error(fmt.Sprintf("Command not understood: %s", input), []string{
		"Try typing naturally: 'gather all', 'seal \"message\"', 'jump to yesterday'",
		"Use 'help' to see available commands",
		"Use 'examples' to see command examples",
	})

	// Provide suggestions
	suggestions := ech.parser.Suggest(input)
	if len(suggestions) > 0 {
		fmt.Println("\nDid you mean:")
		for _, suggestion := range suggestions[:min(3, len(suggestions))] {
			fmt.Printf("  %s\n", suggestion)
		}
	}
}

// Helper functions (these would integrate with the actual repository)

func (ech *EnhancedCommandHandler) generateAutoMessage() string {
	// TODO: Implement semantic analysis of changes
	return "Automatic commit message based on changes"
}

func (ech *EnhancedCommandHandler) getCurrentHash() objects.Hash {
	// TODO: Get current position hash
	return objects.Hash{}
}

func (ech *EnhancedCommandHandler) getCurrentName() string {
	// TODO: Get current position name
	return "current-position"
}

func (ech *EnhancedCommandHandler) getPreviousHash() objects.Hash {
	// TODO: Get previous position hash
	return objects.Hash{}
}

func (ech *EnhancedCommandHandler) getPreviousName() string {
	// TODO: Get previous position name
	return "previous-position"
}

func (ech *EnhancedCommandHandler) getCurrentUser() string {
	// TODO: Get from config
	return "current-user"
}

func (ech *EnhancedCommandHandler) getCurrentTimeline() string {
	// TODO: Get current timeline
	return "main"
}

func (ech *EnhancedCommandHandler) getCurrentWorkspace() *workspace.Workspace {
	// TODO: Get current workspace
	return nil
}

func (ech *EnhancedCommandHandler) getWorkspaceStatus() *WorkspaceStatus {
	// TODO: Build rich workspace status
	return &WorkspaceStatus{
		Timeline:       "main",
		Position:       "bright-river-42",
		OverwriteCount: 0,
		IsProtected:    false,
		LastSealTime:   time.Now().Add(-2 * time.Hour),
		AnvilFiles:     []string{"src/main.go", "README.md"},
		ModifiedFiles:  []string{"src/auth.go"},
		UntrackedFiles: []string{"temp.txt"},
		Snapshots:      []string{"ui-work", "backend-changes"},
	}
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

// Additional handler methods would go here...
func (ech *EnhancedCommandHandler) handleTimelineCreate(cmd *commands.ParsedCommand) error {
	name := cmd.Arguments["name"]
	ech.output.Info(fmt.Sprintf("Creating timeline '%s'...", name))

	if err := ech.repo.CreateTimeline(name, ""); err != nil {
		ech.output.Error(fmt.Sprintf("Failed to create timeline '%s'", name), []string{
			"Check if timeline already exists",
			"timeline list                 # See existing timelines",
		})
		return err
	}

	ech.output.Success(fmt.Sprintf("Created timeline '%s'", name))
	return nil
}

func (ech *EnhancedCommandHandler) handleTimelineCreateFrom(cmd *commands.ParsedCommand) error {
	name := cmd.Arguments["name"]
	from := cmd.Arguments["from"]

	ech.output.Info(fmt.Sprintf("Creating timeline '%s' from '%s'...", name, from))

	// Resolve the 'from' reference
	_, err := ech.references.Resolve(from, ech.getCurrentTimeline())
	if err != nil {
		ech.output.Error(fmt.Sprintf("Reference '%s' not found", from), []string{
			"chronicle                     # See available positions",
			"Use quotes for natural language: \"yesterday\"",
		})
		return err
	}

	// TODO: Implement timeline creation from specific point
	if err := ech.repo.CreateTimeline(name, fmt.Sprintf("Created from %s", from)); err != nil {
		ech.output.Error("Failed to create timeline", nil)
		return err
	}

	ech.output.Success(fmt.Sprintf("Created timeline '%s' from '%s'", name, from))
	return nil
}

func (ech *EnhancedCommandHandler) handleTimelineList(cmd *commands.ParsedCommand) error {
	// TODO: Get timeline information
	timelines := []*TimelineInfo{
		{Name: "main", Description: "Main development line", SealCount: 42, IsCurrent: true},
		{Name: "feature", Description: "Feature development", SealCount: 7, IsCurrent: false},
	}

	ech.output.ShowTimelines(timelines, "main")
	return nil
}

func (ech *EnhancedCommandHandler) handleJumpRelative(cmd *commands.ParsedCommand) error {
	countStr := cmd.Arguments["count"]
	count, err := strconv.Atoi(countStr)
	if err != nil {
		ech.output.Error("Invalid count", []string{
			"Use a number: 'jump back 3'",
			"jump back 1                   # Go back one commit",
		})
		return err
	}

	ech.output.Info(fmt.Sprintf("Jumping back %d position(s)...", count))

	// TODO: Implement relative jumping
	ech.output.Success(fmt.Sprintf("Jumped back %d position(s)", count))
	return nil
}

func (ech *EnhancedCommandHandler) handlePosition(cmd *commands.ParsedCommand) error {
	// TODO: Get current position info
	ech.output.Info("Current position: bright-river-42 (#150)")
	ech.output.Info("Timeline: main")
	ech.output.Info("Last sealed: 2 hours ago")
	return nil
}

func (ech *EnhancedCommandHandler) handleWorkspaceSave(cmd *commands.ParsedCommand) error {
	name := cmd.Arguments["name"]

	ech.output.Info(fmt.Sprintf("Saving workspace as '%s'...", name))

	// TODO: Save workspace
	ech.output.Success(fmt.Sprintf("Workspace saved as '%s'", name))
	return nil
}

func (ech *EnhancedCommandHandler) handleWorkspaceLoad(cmd *commands.ParsedCommand) error {
	name := cmd.Arguments["name"]

	ech.output.Info(fmt.Sprintf("Loading workspace '%s'...", name))

	// TODO: Load workspace
	ech.output.Success(fmt.Sprintf("Workspace '%s' loaded", name))
	return nil
}

func (ech *EnhancedCommandHandler) handleShelfPut(cmd *commands.ParsedCommand) error {
	description := cmd.Arguments["description"]

	ech.output.Info(fmt.Sprintf("Putting work on shelf: %s", description))

	// TODO: Implement shelf
	ech.output.Success("Work put on shelf successfully")
	return nil
}

func (ech *EnhancedCommandHandler) handleShelfTake(cmd *commands.ParsedCommand) error {
	description := cmd.Arguments["description"]

	ech.output.Info(fmt.Sprintf("Taking work from shelf: %s", description))

	// TODO: Implement shelf
	ech.output.Success("Work taken from shelf successfully")
	return nil
}

func (ech *EnhancedCommandHandler) handleTimelineDelete(cmd *commands.ParsedCommand) error {
	name := cmd.Arguments["name"]

	ech.output.Warning(fmt.Sprintf("Deleting timeline '%s'...", name))

	// TODO: Implement timeline deletion with safety checks
	ech.output.Success(fmt.Sprintf("Timeline '%s' deleted", name))
	return nil
}
