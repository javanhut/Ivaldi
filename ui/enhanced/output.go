package enhanced

import (
	"fmt"
	"strings"
	"time"
)

// Colors and symbols for rich output
const (
	// Colors
	ColorReset  = "\033[0m"
	ColorRed    = "\033[31m"
	ColorGreen  = "\033[32m"
	ColorYellow = "\033[33m"
	ColorBlue   = "\033[34m"
	ColorPurple = "\033[35m"
	ColorCyan   = "\033[36m"
	ColorGray   = "\033[37m"
	ColorBold   = "\033[1m"
	ColorDim    = "\033[2m"
	
	// Symbols
	SymbolSuccess    = "✓"
	SymbolError      = "✗"
	SymbolWarning    = "!"
	SymbolInfo       = "i"
	SymbolPosition   = "*"
	SymbolForge      = "*"
	SymbolSeal       = "*"
	SymbolTimeline   = "*"
	SymbolPortal     = "*"
	SymbolOverwrite  = "*"
	SymbolProtected  = "*"
	SymbolCollaborate = "*"
)

// OutputStyle controls visual styling
type OutputStyle struct {
	UseColors    bool
	UseEmoji     bool
	UseProgress  bool
	RelativeTime bool
	Verbose      bool
}

// EnhancedOutput provides rich visual CLI output
type EnhancedOutput struct {
	style *OutputStyle
}

func NewEnhancedOutput() *EnhancedOutput {
	return &EnhancedOutput{
		style: &OutputStyle{
			UseColors:    true,
			UseEmoji:     true,
			UseProgress:  true,
			RelativeTime: true,
			Verbose:      false,
		},
	}
}

// Success prints a success message
func (eo *EnhancedOutput) Success(message string) {
	if eo.style.UseEmoji {
		fmt.Printf("%s ", SymbolSuccess)
	}
	if eo.style.UseColors {
		fmt.Printf("%s%s%s\n", ColorGreen, message, ColorReset)
	} else {
		fmt.Printf("%s\n", message)
	}
}

// Error prints an error with helpful guidance
func (eo *EnhancedOutput) Error(message string, solutions []string) {
	if eo.style.UseEmoji {
		fmt.Printf("%s ", SymbolError)
	}
	if eo.style.UseColors {
		fmt.Printf("%s%s%s\n", ColorRed, message, ColorReset)
	} else {
		fmt.Printf("Error: %s\n", message)
	}
	
	if len(solutions) > 0 {
		fmt.Println()
		fmt.Println("Your options:")
		for _, solution := range solutions {
			if eo.style.UseColors {
				fmt.Printf("  %s→%s %s\n", ColorCyan, ColorReset, solution)
			} else {
				fmt.Printf("  → %s\n", solution)
			}
		}
	}
}

// Warning prints a warning message
func (eo *EnhancedOutput) Warning(message string) {
	if eo.style.UseEmoji {
		fmt.Printf("%s ", SymbolWarning)
	}
	if eo.style.UseColors {
		fmt.Printf("%s%s%s\n", ColorYellow, message, ColorReset)
	} else {
		fmt.Printf("Warning: %s\n", message)
	}
}

// Info prints an informational message
func (eo *EnhancedOutput) Info(message string) {
	if eo.style.UseColors {
		fmt.Printf("%s%s%s\n", ColorBlue, message, ColorReset)
	} else {
		fmt.Printf("%s\n", message)
	}
}

// FileAdded prints a file status as added (green)
func (eo *EnhancedOutput) FileAdded(filename string) {
	if eo.style.UseColors {
		fmt.Printf("  %s[added]%s    %s\n", ColorGreen, ColorReset, filename)
	} else {
		fmt.Printf("  [added]    %s\n", filename)
	}
}

// FileModified prints a file status as modified (blue)
func (eo *EnhancedOutput) FileModified(filename string) {
	if eo.style.UseColors {
		fmt.Printf("  %s[modified]%s %s\n", ColorBlue, ColorReset, filename)
	} else {
		fmt.Printf("  [modified] %s\n", filename)
	}
}

// FileDeleted prints a file status as deleted (red)
func (eo *EnhancedOutput) FileDeleted(filename string) {
	if eo.style.UseColors {
		fmt.Printf("  %s[deleted]%s  %s\n", ColorRed, ColorReset, filename)
	} else {
		fmt.Printf("  [deleted]  %s\n", filename)
	}
}

// FileChanged prints a file status as changed (blue, for unstaged)
func (eo *EnhancedOutput) FileChanged(filename string) {
	if eo.style.UseColors {
		fmt.Printf("  %s[changed]%s  %s\n", ColorBlue, ColorReset, filename)
	} else {
		fmt.Printf("  [changed]  %s\n", filename)
	}
}

// ShowWorkspaceStatus displays rich workspace status
func (eo *EnhancedOutput) ShowWorkspaceStatus(status *WorkspaceStatus) {
	// Header
	if eo.style.UseEmoji {
		fmt.Printf("%s ", SymbolPosition)
	}
	if eo.style.UseColors {
		fmt.Printf("%sWorkspace Status%s\n", ColorBold, ColorReset)
	} else {
		fmt.Println("Workspace Status")
	}
	
	// Timeline and position
	fmt.Printf("Timeline: %s\n", eo.colorize(status.Timeline, ColorCyan))
	fmt.Printf("Position: %s", eo.colorize(status.Position, ColorBlue))
	
	if status.OverwriteCount > 0 {
		if eo.style.UseEmoji {
			fmt.Printf(" %s%d", SymbolOverwrite, status.OverwriteCount)
		} else {
			fmt.Printf(" (♻%d)", status.OverwriteCount)
		}
	}
	
	if status.IsProtected {
		if eo.style.UseEmoji {
			fmt.Printf(" %s", SymbolProtected)
		} else {
			fmt.Printf(" [PROTECTED]")
		}
	}
	fmt.Println()
	
	// Time information
	if eo.style.RelativeTime && !status.LastSealTime.IsZero() {
		fmt.Printf("Last sealed: %s\n", eo.relativeTime(status.LastSealTime))
	}
	
	// Anvil files
	if len(status.AnvilFiles) > 0 {
		fmt.Println()
		if eo.style.UseEmoji {
			fmt.Printf("%s ", SymbolForge)
		}
		fmt.Printf("On the anvil:\n")
		for _, file := range status.AnvilFiles {
			fmt.Printf("  %s %s\n", eo.colorize("gathered:", ColorGreen), file)
		}
	}
	
	// Modified files
	if len(status.ModifiedFiles) > 0 {
		fmt.Println()
		fmt.Printf("Changes not on anvil:\n")
		for _, file := range status.ModifiedFiles {
			fmt.Printf("  %s %s\n", eo.colorize("modified:", ColorYellow), file)
		}
	}
	
	// Untracked files
	if len(status.UntrackedFiles) > 0 {
		fmt.Println()
		fmt.Printf("Untracked files:\n")
		for _, file := range status.UntrackedFiles {
			fmt.Printf("  %s\n", eo.colorize(file, ColorGray))
		}
	}
	
	// Workspace snapshots
	if len(status.Snapshots) > 0 {
		fmt.Println()
		fmt.Printf("Preserved workspaces:\n")
		for _, snapshot := range status.Snapshots {
			fmt.Printf("  %s (%s)\n", snapshot, eo.colorize("saved", ColorBlue))
		}
	}
}

// ShowSealHistory displays rich commit history
func (eo *EnhancedOutput) ShowSealHistory(seals []*SealInfo) {
	if eo.style.UseEmoji {
		fmt.Printf("%s ", SymbolSeal)
	}
	if eo.style.UseColors {
		fmt.Printf("%sSealing Chronicle%s\n\n", ColorBold, ColorReset)
	} else {
		fmt.Println("Sealing Chronicle\n")
	}
	
	for i, seal := range seals {
		// Seal name with overwrite indicator
		sealName := seal.Name
		if seal.OverwriteCount > 0 {
			if eo.style.UseEmoji {
				sealName += fmt.Sprintf(" %s%d", SymbolOverwrite, seal.OverwriteCount)
			} else {
				sealName += fmt.Sprintf(" ♻%d", seal.OverwriteCount)
			}
		}
		
		// Protection indicator
		if seal.IsProtected {
			if eo.style.UseEmoji {
				sealName += fmt.Sprintf(" %s", SymbolProtected)
			} else {
				sealName += " [PROTECTED]"
			}
		}
		
		fmt.Printf("%s (#%d) - %s\n", 
			eo.colorize(sealName, ColorBlue),
			seal.Iteration,
			seal.Message,
		)
		
		// Author and time
		timeStr := seal.Timestamp.Format("2006-01-02 15:04:05")
		if eo.style.RelativeTime {
			timeStr = eo.relativeTime(seal.Timestamp)
		}
		
		fmt.Printf("  %s <%s> - %s\n",
			eo.colorize(seal.Author.Name, ColorCyan),
			eo.colorize(seal.Author.Email, ColorGray),
			eo.colorize(timeStr, ColorGray),
		)
		
		// Add spacing between entries
		if i < len(seals)-1 {
			fmt.Println()
		}
	}
}

// ShowTimelines displays timeline information
func (eo *EnhancedOutput) ShowTimelines(timelines []*TimelineInfo, current string) {
	if eo.style.UseEmoji {
		fmt.Printf("%s ", SymbolTimeline)
	}
	if eo.style.UseColors {
		fmt.Printf("%sTimelines%s\n", ColorBold, ColorReset)
	} else {
		fmt.Println("Timelines")
	}
	
	for _, timeline := range timelines {
		marker := " "
		nameColor := ColorReset
		
		if timeline.Name == current {
			marker = "*"
			nameColor = ColorGreen
		}
		
		fmt.Printf("%s %s", marker, eo.colorize(timeline.Name, nameColor))
		
		if timeline.Description != "" {
			fmt.Printf(" - %s", eo.colorize(timeline.Description, ColorGray))
		}
		
		fmt.Printf(" (%d seals)\n", timeline.SealCount)
	}
}

// ShowPortals displays portal information
func (eo *EnhancedOutput) ShowPortals(portals map[string]*PortalInfo) {
	if eo.style.UseEmoji {
		fmt.Printf("%s ", SymbolPortal)
	}
	if eo.style.UseColors {
		fmt.Printf("%sPortals%s\n", ColorBold, ColorReset)
	} else {
		fmt.Println("Portals")
	}
	
	if len(portals) == 0 {
		fmt.Printf("  %s\n", eo.colorize("No portals configured", ColorGray))
		return
	}
	
	for name, portal := range portals {
		status := ""
		statusColor := ColorGray
		
		switch portal.Status {
		case "connected":
			status = "✓ connected"
			statusColor = ColorGreen
		case "disconnected":
			status = "✗ disconnected"
			statusColor = ColorRed
		case "syncing":
			status = "⟳ syncing"
			statusColor = ColorYellow
		default:
			status = "? unknown"
		}
		
		fmt.Printf("  %s -> %s [%s]\n",
			eo.colorize(name, ColorCyan),
			portal.URL,
			eo.colorize(status, statusColor),
		)
		
		if portal.LastSync != nil && !portal.LastSync.IsZero() {
			fmt.Printf("    Last sync: %s\n", eo.relativeTime(*portal.LastSync))
		}
	}
}

// Progress shows a progress bar for long operations
func (eo *EnhancedOutput) Progress(current, total int, message string) {
	if !eo.style.UseProgress {
		return
	}
	
	percentage := float64(current) / float64(total) * 100
	barLength := 40
	filled := int(float64(barLength) * float64(current) / float64(total))
	
	bar := strings.Repeat("█", filled) + strings.Repeat("░", barLength-filled)
	
	fmt.Printf("\r%s [%s] %3.0f%% (%d/%d)",
		message, bar, percentage, current, total)
	
	if current == total {
		fmt.Println() // New line when complete
	}
}

// Helper functions

func (eo *EnhancedOutput) colorize(text, color string) string {
	if eo.style.UseColors {
		return color + text + ColorReset
	}
	return text
}

func (eo *EnhancedOutput) relativeTime(t time.Time) string {
	duration := time.Since(t)
	
	if duration < time.Minute {
		return "just now"
	} else if duration < time.Hour {
		minutes := int(duration.Minutes())
		return fmt.Sprintf("%d minute%s ago", minutes, pluralize(minutes))
	} else if duration < 24*time.Hour {
		hours := int(duration.Hours())
		return fmt.Sprintf("%d hour%s ago", hours, pluralize(hours))
	} else if duration < 7*24*time.Hour {
		days := int(duration.Hours() / 24)
		return fmt.Sprintf("%d day%s ago", days, pluralize(days))
	} else if duration < 30*24*time.Hour {
		weeks := int(duration.Hours() / (24 * 7))
		return fmt.Sprintf("%d week%s ago", weeks, pluralize(weeks))
	} else {
		months := int(duration.Hours() / (24 * 30))
		return fmt.Sprintf("%d month%s ago", months, pluralize(months))
	}
}

func pluralize(n int) string {
	if n == 1 {
		return ""
	}
	return "s"
}

// Data structures for display

type WorkspaceStatus struct {
	Timeline       string
	Position       string
	OverwriteCount int
	IsProtected    bool
	LastSealTime   time.Time
	AnvilFiles     []string
	ModifiedFiles  []string
	UntrackedFiles []string
	Snapshots      []string
}

type SealInfo struct {
	Name           string
	Iteration      int
	Message        string
	Author         struct {
		Name  string
		Email string
	}
	Timestamp      time.Time
	OverwriteCount int
	IsProtected    bool
}

type TimelineInfo struct {
	Name        string
	Description string
	SealCount   int
	IsCurrent   bool
}

type PortalInfo struct {
	URL      string
	Status   string
	LastSync *time.Time
}