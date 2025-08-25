package commands

import (
	"fmt"
	"regexp"
	"strings"
)

// NaturalLanguageParser converts natural language commands to structured operations
type NaturalLanguageParser struct {
	patterns map[string]*CommandPattern
}

type CommandPattern struct {
	Regex   *regexp.Regexp
	Handler string
	Groups  []string
}

type ParsedCommand struct {
	Command   string
	Arguments map[string]string
	Original  string
}

func NewNaturalLanguageParser() *NaturalLanguageParser {
	parser := &NaturalLanguageParser{
		patterns: make(map[string]*CommandPattern),
	}

	parser.initializePatterns()
	return parser
}

func (nlp *NaturalLanguageParser) initializePatterns() {
	// Basic operations
	nlp.addPattern("gather_all", `^gather all$`, "gather", []string{})
	nlp.addPattern("gather_except", `^gather (.+) except (.+)$`, "gather_except", []string{"include", "exclude"})
	nlp.addPattern("gather_files", `^gather (.+)$`, "gather", []string{"files"})

	// Sealing operations
	nlp.addPattern("seal_auto", `^seal$`, "seal_auto", []string{})
	nlp.addPattern("seal_message", `^seal -m "(.+)"$`, "seal", []string{"message"})
	nlp.addPattern("seal_message_alt", `^seal "(.+)"$`, "seal", []string{"message"})
	nlp.addPattern("unseal", `^unseal$`, "unseal", []string{})

	// Timeline operations
	nlp.addPattern("timeline_create", `^timeline create (\w+)$`, "timeline_create", []string{"name"})
	nlp.addPattern("timeline_create_from", `^timeline create (\w+) --from "(.+)"$`, "timeline_create_from", []string{"name", "from"})
	nlp.addPattern("timeline_switch", `^timeline switch (\w+)$`, "timeline_switch", []string{"name"})
	nlp.addPattern("timeline_delete", `^timeline delete (\w+)$`, "timeline_delete", []string{"name"})
	nlp.addPattern("timeline_rename", `^timeline rename (\w+) (\w+)$`, "timeline_rename", []string{"old", "new"})
	nlp.addPattern("timeline_list", `^timeline list$`, "timeline_list", []string{})

	// Position navigation
	nlp.addPattern("jump_to_natural", `^jump to "(.+)"$`, "jump", []string{"reference"})
	nlp.addPattern("jump_to_reference", `^jump to (.+)$`, "jump", []string{"reference"})
	nlp.addPattern("jump_back", `^jump back (\d+)$`, "jump_relative", []string{"count"})
	nlp.addPattern("jump_forward", `^jump forward (\d+)$`, "jump_relative", []string{"count"})
	nlp.addPattern("position", `^position$`, "position", []string{})
	nlp.addPattern("trail", `^trail$`, "trail", []string{})

	// Workspace operations
	nlp.addPattern("workspace_status", `^workspace$`, "workspace_status", []string{})
	nlp.addPattern("workspace_save", `^workspace save "(.+)"$`, "workspace_save", []string{"name"})
	nlp.addPattern("workspace_load", `^workspace load "(.+)"$`, "workspace_load", []string{"name"})
	nlp.addPattern("workspace_clean", `^workspace clean$`, "workspace_clean", []string{})

	// Shelf operations
	nlp.addPattern("shelf_put", `^shelf put "(.+)"$`, "shelf_put", []string{"description"})
	nlp.addPattern("shelf_take", `^shelf take "(.+)"$`, "shelf_take", []string{"description"})
	nlp.addPattern("shelf_list", `^shelf list$`, "shelf_list", []string{})

	// Collaboration
	nlp.addPattern("portal_add", `^portal add (\w+) (.+)$`, "portal_add", []string{"name", "url"})
	nlp.addPattern("sync_portal", `^sync (\w+)$`, "sync", []string{"portal"})
	nlp.addPattern("collaborate_start", `^collaborate start$`, "collaborate_start", []string{})
	nlp.addPattern("collaborate_join", `^collaborate join (.+)$`, "collaborate_join", []string{"session"})
	nlp.addPattern("mesh_start", `^mesh start$`, "mesh_start", []string{})
	nlp.addPattern("mesh_sync", `^mesh sync (.+)$`, "mesh_sync", []string{"peer"})

	// Advanced operations
	nlp.addPattern("fuse_timelines", `^fuse (\w+) into (\w+)$`, "fuse", []string{"source", "target"})
	nlp.addPattern("reshape_last", `^reshape last (\d+)$`, "reshape", []string{"count"})
	nlp.addPattern("pluck_commit", `^pluck "(.+)"$`, "pluck", []string{"reference"})
	nlp.addPattern("hunt_bug", `^hunt "(.+)"$`, "hunt", []string{"query"})
	nlp.addPattern("trace_line", `^trace (.+):(\d+)$`, "trace", []string{"file", "line"})
	nlp.addPattern("chronicle_rich", `^chronicle --rich$`, "chronicle_rich", []string{})
	nlp.addPattern("chronicle", `^chronicle$`, "chronicle", []string{})
	nlp.addPattern("find_commits", `^find "(.+)"$`, "find", []string{"query"})
	nlp.addPattern("compare_ranges", `^compare (.+) (.+)$`, "compare", []string{"from", "to"})

	// Natural language queries
	nlp.addPattern("when_added", `^when was (.+) added$`, "find_when_added", []string{"item"})
	nlp.addPattern("who_changed", `^who changed (.+)$`, "find_who_changed", []string{"item"})
	nlp.addPattern("what_changed", `^what changed in (.+)$`, "find_what_changed", []string{"item"})
	nlp.addPattern("last_change", `^last change to (.+)$`, "find_last_change", []string{"item"})

	// Protection operations
	nlp.addPattern("protect_commit", `^protect (.+)$`, "protect", []string{"reference"})
	nlp.addPattern("unprotect_commit", `^unprotect (.+)$`, "unprotect", []string{"reference"})

	// Archive operations
	nlp.addPattern("show_overwrites", `^show overwrites for (.+)$`, "show_overwrites", []string{"reference"})
	nlp.addPattern("show_archived", `^show archived versions of (.+)$`, "show_archived", []string{"reference"})
	nlp.addPattern("restore_version", `^restore (.+) version (\d+)$`, "restore_version", []string{"reference", "version"})
}

func (nlp *NaturalLanguageParser) addPattern(name, pattern, handler string, groups []string) {
	regex := regexp.MustCompile(`(?i)` + pattern) // Case insensitive
	nlp.patterns[name] = &CommandPattern{
		Regex:   regex,
		Handler: handler,
		Groups:  groups,
	}
}

// Parse converts natural language input to structured command
func (nlp *NaturalLanguageParser) Parse(input string) (*ParsedCommand, error) {
	input = strings.TrimSpace(input)

	for _, pattern := range nlp.patterns {
		matches := pattern.Regex.FindStringSubmatch(input)
		if matches != nil {
			cmd := &ParsedCommand{
				Command:   pattern.Handler,
				Arguments: make(map[string]string),
				Original:  input,
			}

			// Extract named groups
			for i, group := range pattern.Groups {
				if i+1 < len(matches) {
					cmd.Arguments[group] = matches[i+1]
				}
			}

			return cmd, nil
		}
	}

	return nil, fmt.Errorf("command not understood: %s", input)
}

// Suggest provides command suggestions based on partial input
func (nlp *NaturalLanguageParser) Suggest(partial string) []string {
	var suggestions []string
	partial = strings.ToLower(strings.TrimSpace(partial))

	// Common command starters
	starters := map[string][]string{
		"gather": {
			"gather all",
			"gather src/ except tests/",
			"gather *.go",
		},
		"seal": {
			"seal",
			"seal \"commit message\"",
		},
		"timeline": {
			"timeline create feature",
			"timeline switch main",
			"timeline list",
		},
		"jump": {
			"jump to \"yesterday\"",
			"jump back 3",
			"jump to #150",
		},
		"workspace": {
			"workspace",
			"workspace save \"ui-changes\"",
			"workspace load \"ui-changes\"",
			"workspace clean",
		},
		"shelf": {
			"shelf put \"work in progress\"",
			"shelf take \"work in progress\"",
			"shelf list",
		},
		"fuse": {
			"fuse feature into main",
		},
		"find": {
			"find \"authentication\"",
			"find \"commits about database\"",
		},
		"when": {
			"when was authentication added",
		},
		"who": {
			"who changed this function",
		},
		"show": {
			"show overwrites for bright-river-42",
			"show archived versions of main#150",
		},
	}

	// Find matching starters
	for starter, commands := range starters {
		if strings.HasPrefix(starter, partial) || strings.HasPrefix(partial, starter) {
			suggestions = append(suggestions, commands...)
		}
	}

	// If no specific matches, show general patterns
	if len(suggestions) == 0 {
		suggestions = []string{
			"gather all",
			"seal \"message\"",
			"timeline switch main",
			"jump to \"yesterday\"",
			"workspace save \"name\"",
			"find \"search term\"",
		}
	}

	return suggestions
}

// Explain provides human-readable explanation of what a command does
func (nlp *NaturalLanguageParser) Explain(command string) string {
	explanations := map[string]string{
		"gather":          "Gather files to the anvil (staging area) for sealing",
		"gather_except":   "Gather specific files while excluding others",
		"seal":            "Seal changes into history with a message",
		"seal_auto":       "Seal changes with an auto-generated message",
		"unseal":          "Undo the last seal (like git reset --soft HEAD~1)",
		"timeline_create": "Create a new timeline (branch) for development",
		"timeline_switch": "Switch to a different timeline, preserving your work",
		"jump":            "Jump to any position in history using natural references",
		"workspace_save":  "Save current workspace state with a name",
		"workspace_load":  "Load a previously saved workspace state",
		"shelf_put":       "Put work on the shelf temporarily",
		"shelf_take":      "Take work from the shelf",
		"fuse":            "Fuse two timelines together (merge)",
		"pluck":           "Pluck specific changes from another timeline",
		"hunt":            "Hunt for bugs using binary search",
		"find":            "Find commits containing specific terms",
		"trace":           "Trace the history of a specific line",
		"protect":         "Protect a commit from being overwritten",
		"show_overwrites": "Show all times a commit has been overwritten",
	}

	if explanation, exists := explanations[command]; exists {
		return explanation
	}

	return "Command explanation not available"
}

// Examples provides usage examples for commands
func (nlp *NaturalLanguageParser) Examples() map[string][]string {
	return map[string][]string{
		"Basic Operations": {
			"gather all",
			"gather src/ except tests/",
			"seal \"Add user authentication\"",
			"unseal",
		},
		"Timeline Management": {
			"timeline create feature",
			"timeline switch main",
			"timeline list",
			"fuse feature into main",
		},
		"Navigation": {
			"jump to \"yesterday\"",
			"jump back 3",
			"jump to bright-river-42",
			"jump to main#150",
		},
		"Workspace": {
			"workspace save \"ui-work\"",
			"workspace load \"ui-work\"",
			"shelf put \"work in progress\"",
			"shelf take \"work in progress\"",
		},
		"Search & History": {
			"find \"authentication\"",
			"when was login added",
			"who changed src/auth.go",
			"trace src/main.go:45",
		},
		"Collaboration": {
			"portal add team https://github.com/team/repo",
			"sync team",
			"collaborate start",
			"mesh sync alice",
		},
	}
}
