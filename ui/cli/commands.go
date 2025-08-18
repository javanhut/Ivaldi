package cli

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/spf13/cobra"
	"ivaldi/forge"
)

var forgeCmd = &cobra.Command{
	Use:   "forge [directory]",
	Short: "Initialize a new Ivaldi repository",
	Long:  "Creates a new Ivaldi repository in the specified directory (current directory if not specified)",
	Run: func(cmd *cobra.Command, args []string) {
		dir := "."
		if len(args) > 0 {
			dir = args[0]
		}

		absDir, err := filepath.Abs(dir)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Initialize(absDir)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error initializing repository: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Initialized empty Ivaldi repository in %s\n", repo.Root())
		fmt.Println("Your workspace is ready for crafting!")
	},
}

var mirrorCmd = &cobra.Command{
	Use:   "mirror <url> [directory]",
	Short: "Mirror a repository from a remote portal",
	Long:  "Create a local mirror of a repository from GitHub or other portals",
	Args:  cobra.MinimumNArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		url := args[0]
		dir := ""
		
		if len(args) > 1 {
			dir = args[1]
		} else {
			// Extract repository name from URL
			parts := strings.Split(url, "/")
			if len(parts) > 0 {
				repoName := parts[len(parts)-1]
				repoName = strings.TrimSuffix(repoName, ".git")
				dir = repoName
			} else {
				dir = "repository"
			}
		}

		absDir, err := filepath.Abs(dir)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		// Check if directory already exists
		if _, err := os.Stat(absDir); !os.IsNotExist(err) {
			fmt.Fprintf(os.Stderr, "Error: directory '%s' already exists\n", dir)
			os.Exit(1)
		}

		fmt.Printf("Mirroring repository from %s...\n", url)
		
		repo, err := forge.Mirror(url, absDir)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error copying repository: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Repository copied to %s\n", repo.Root())
		fmt.Println("Ready to craft! Use 'ivaldi status' to see the current state.")
	},
}

var gatherCmd = &cobra.Command{
	Use:   "gather [files...]",
	Short: "Gather changes to the anvil (staging area)",
	Long:  "Gather specified files to the anvil where they will be prepared for sealing",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if len(args) == 0 {
			args = []string{"."}
		}

		if err := repo.Gather(args); err != nil {
			fmt.Fprintf(os.Stderr, "Error gathering files: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Gathered %d file(s) to the anvil\n", len(args))
	},
}

var discardCmd = &cobra.Command{
	Use:   "discard [files...]",
	Short: "Discard files from the anvil",
	Long:  "Remove specified files from the anvil, or discard all changes if no files specified",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		all, _ := cmd.Flags().GetBool("all")
		
		if all || len(args) == 0 {
			// Discard everything from anvil
			count := repo.DiscardAll()
			if count == 0 {
				fmt.Println("Nothing on the anvil to discard")
			} else {
				fmt.Printf("Discarded %d file(s) from the anvil\n", count)
			}
		} else {
			// Discard specific files
			count, err := repo.Discard(args)
			if err != nil {
				fmt.Fprintf(os.Stderr, "Error discarding files: %v\n", err)
				os.Exit(1)
			}
			
			if count == 0 {
				fmt.Println("No matching files found on the anvil")
			} else {
				fmt.Printf("Discarded %d file(s) from the anvil\n", count)
			}
		}
	},
}

var sealCmd = &cobra.Command{
	Use:   "seal",
	Short: "Seal changes into history",
	Long:  "Create a new seal (commit) with the changes gathered on the anvil",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		message, _ := cmd.Flags().GetString("message")
		if message == "" {
			fmt.Fprintf(os.Stderr, "Error: seal message is required (-m flag)\n")
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		seal, err := repo.Seal(message)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error creating seal: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Created seal: %s\n", seal.Name)
		fmt.Printf("Message: %s\n", seal.Message)
	},
}

var timelineCmd = &cobra.Command{
	Use:   "timeline",
	Short: "Manage timelines (branches)",
	Long:  "Create, switch between, and manage development timelines",
}

var timelineListCmd = &cobra.Command{
	Use:   "list",
	Short: "List all timelines",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		timelines := repo.ListTimelines()
		current := repo.CurrentTimeline()

		fmt.Println("Timelines:")
		for _, timeline := range timelines {
			marker := " "
			if timeline.Name == current {
				marker = "*"
			}
			fmt.Printf("%s %s - %s\n", marker, timeline.Name, timeline.Description)
		}
	},
}

var timelineCreateCmd = &cobra.Command{
	Use:   "create <name> [description]",
	Short: "Create a new timeline",
	Args:  cobra.MinimumNArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		name := args[0]
		description := ""
		if len(args) > 1 {
			description = strings.Join(args[1:], " ")
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if err := repo.CreateTimeline(name, description); err != nil {
			fmt.Fprintf(os.Stderr, "Error creating timeline: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Created timeline: %s\n", name)
	},
}

var timelineSwitchCmd = &cobra.Command{
	Use:   "switch <name>",
	Short: "Switch to a different timeline",
	Args:  cobra.ExactArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		name := args[0]

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if err := repo.SwitchTimeline(name); err != nil {
			fmt.Fprintf(os.Stderr, "Error switching timeline: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Switched to timeline: %s\n", name)
	},
}

var jumpCmd = &cobra.Command{
	Use:   "jump <reference>",
	Short: "Jump to a specific position in history",
	Long:  "Jump to any position using natural language, iteration numbers, or memorable names",
	Args:  cobra.ExactArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		reference := args[0]

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if err := repo.Jump(reference); err != nil {
			fmt.Fprintf(os.Stderr, "Error jumping to position: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Jumped to: %s\n", reference)
	},
}

var statusCmd = &cobra.Command{
	Use:   "status",
	Short: "Show workspace status",
	Long:  "Display the current state of your workspace, anvil, and position",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		status := repo.Status()
		
		fmt.Printf("Timeline: %s\n", status.Timeline)
		fmt.Printf("Position: %s\n", status.Position)
		
		if len(status.Gathered) > 0 {
			fmt.Println("\nOn the anvil:")
			for _, file := range status.Gathered {
				fmt.Printf("  gathered: %s\n", file)
			}
		}
		
		if len(status.Modified) > 0 {
			fmt.Println("\nChanges not on anvil:")
			for _, file := range status.Modified {
				fmt.Printf("  modified: %s\n", file)
			}
		}
		
		if len(status.Untracked) > 0 {
			fmt.Println("\nUntracked files:")
			for _, file := range status.Untracked {
				fmt.Printf("  %s\n", file)
			}
		}
	},
}

var logCmd = &cobra.Command{
	Use:   "log",
	Short: "Show seal history",
	Long:  "Display the history of seals with their memorable names and messages",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		seals, err := repo.History(10)
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error getting history: %v\n", err)
			os.Exit(1)
		}

		for _, seal := range seals {
			overwriteIndicator := ""
			if len(seal.Overwrites) > 0 {
				overwriteIndicator = fmt.Sprintf(" ♻%d", len(seal.Overwrites))
			}
			
			fmt.Printf("%s%s (#%d) - %s\n", 
				seal.Name, overwriteIndicator, seal.Iteration, seal.Message)
			fmt.Printf("  %s <%s> - %s\n\n", 
				seal.Author.Name, seal.Author.Email, seal.Timestamp.Format("2006-01-02 15:04:05"))
		}
	},
}

var fuseCmd = &cobra.Command{
	Use:   "fuse <timeline> into <target>",
	Short: "Fuse one timeline into another",
	Long:  "Merge changes from one timeline into another",
	Run: func(cmd *cobra.Command, args []string) {
		fmt.Println("Fuse command not yet implemented")
	},
}

var reshapeCmd = &cobra.Command{
	Use:   "reshape",
	Short: "Reshape recent history",
	Long:  "Interactively reshape the last few seals",
	Run: func(cmd *cobra.Command, args []string) {
		fmt.Println("Reshape command not yet implemented")
	},
}

var pluckCmd = &cobra.Command{
	Use:   "pluck <reference>",
	Short: "Pluck a specific seal from another timeline",
	Long:  "Cherry-pick a seal from another timeline",
	Run: func(cmd *cobra.Command, args []string) {
		fmt.Println("Pluck command not yet implemented")
	},
}

var shelfCmd = &cobra.Command{
	Use:   "shelf",
	Short: "Manage shelved work",
	Long:  "Save and restore work in progress",
}

var portalCmd = &cobra.Command{
	Use:   "portal",
	Short: "Manage remote portals",
	Long:  "Connect to and sync with remote repositories",
}

var portalAddCmd = &cobra.Command{
	Use:   "add <name> <url>",
	Short: "Add a remote portal",
	Args:  cobra.ExactArgs(2),
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		name := args[0]
		url := args[1]

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if err := repo.AddPortal(name, url); err != nil {
			fmt.Fprintf(os.Stderr, "Error adding portal: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Added portal '%s' -> %s\n", name, url)
	},
}

var portalListCmd = &cobra.Command{
	Use:   "list",
	Short: "List remote portals",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		portals := repo.ListPortals()
		
		if len(portals) == 0 {
			fmt.Println("No portals configured")
			return
		}

		fmt.Println("Configured portals:")
		for name, url := range portals {
			fmt.Printf("  %s -> %s\n", name, url)
		}
	},
}

var portalRemoveCmd = &cobra.Command{
	Use:   "remove <name>",
	Short: "Remove a remote portal",
	Args:  cobra.ExactArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		name := args[0]

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if err := repo.RemovePortal(name); err != nil {
			fmt.Fprintf(os.Stderr, "Error removing portal: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Removed portal '%s'\n", name)
	},
}

var versionCmd = &cobra.Command{
	Use:   "version",
	Short: "Manage version tags",
	Long:  "Create and manage semantic version tags for releases",
}

var versionCreateCmd = &cobra.Command{
	Use:   "create <version> [message]",
	Short: "Create a new version tag",
	Args:  cobra.MinimumNArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		version := args[0]
		message := ""
		if len(args) > 1 {
			message = strings.Join(args[1:], " ")
		} else {
			message = fmt.Sprintf("Release %s", version)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if err := repo.CreateVersion(version, message); err != nil {
			fmt.Fprintf(os.Stderr, "Error creating version: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Created version %s: %s\n", version, message)
	},
}

var versionListCmd = &cobra.Command{
	Use:   "list",
	Short: "List all versions",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		versions := repo.ListVersions()
		
		if len(versions) == 0 {
			fmt.Println("No versions created yet")
			return
		}

		fmt.Println("Versions:")
		for _, v := range versions {
			fmt.Printf("  %s - %s (%s)\n", v.Tag, v.Message, v.Date.Format("2006-01-02"))
		}
	},
}

var versionPushCmd = &cobra.Command{
	Use:   "push <version>",
	Short: "Push version tag to GitHub",
	Args:  cobra.MinimumNArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		version := args[0]
		all, _ := cmd.Flags().GetBool("all")

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		if all {
			fmt.Println("Pushing all versions to GitHub...")
			if err := repo.PushAllVersions(); err != nil {
				fmt.Fprintf(os.Stderr, "Error pushing versions: %v\n", err)
				os.Exit(1)
			}
			fmt.Println("All versions pushed successfully!")
		} else {
			fmt.Printf("Pushing version %s to GitHub...\n", version)
			if err := repo.PushVersion(version); err != nil {
				fmt.Fprintf(os.Stderr, "Error pushing version: %v\n", err)
				os.Exit(1)
			}
			fmt.Printf("Version %s pushed successfully!\n", version)
		}
	},
}

var scoutCmd = &cobra.Command{
	Use:   "scout [portal]",
	Short: "Scout for updates from remote portal",
	Long:  "Check for new changes from remote portal without merging them",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		portalName := "origin"
		if len(args) > 0 {
			portalName = args[0]
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		fmt.Printf("Scouting portal '%s'...\n", portalName)
		if err := repo.Scout(portalName); err != nil {
			fmt.Fprintf(os.Stderr, "Error scouting: %v\n", err)
			os.Exit(1)
		}

		fmt.Println("Scouting complete! Use 'ivaldi sync --pull' to merge changes.")
	},
}

var syncCmd = &cobra.Command{
	Use:   "sync [portal]",
	Short: "Synchronize with remote portal",
	Long:  "Pull and push changes to/from a remote portal",
	Run: func(cmd *cobra.Command, args []string) {
		if err := checkRepo(); err != nil {
			fmt.Fprintf(os.Stderr, "Error: %v\n", err)
			os.Exit(1)
		}

		portalName := "origin"
		if len(args) > 0 {
			portalName = args[0]
		}

		repo, err := forge.Open(".")
		if err != nil {
			fmt.Fprintf(os.Stderr, "Error opening repository: %v\n", err)
			os.Exit(1)
		}

		push, _ := cmd.Flags().GetBool("push")
		pull, _ := cmd.Flags().GetBool("pull")
		
		// Default to both if neither specified
		if !push && !pull {
			push = true
			pull = true
		}

		if pull {
			fmt.Printf("Pulling from portal '%s'...\n", portalName)
			if err := repo.Pull(portalName); err != nil {
				fmt.Fprintf(os.Stderr, "Error pulling: %v\n", err)
				os.Exit(1)
			}
		}

		if push {
			fmt.Printf("Pushing to portal '%s'...\n", portalName)
			if err := repo.Push(portalName); err != nil {
				fmt.Fprintf(os.Stderr, "Error pushing: %v\n", err)
				os.Exit(1)
			}
		}

		fmt.Println("Sync complete!")
	},
}

func init() {
	sealCmd.Flags().StringP("message", "m", "", "Seal message")
	
	discardCmd.Flags().Bool("all", false, "Discard all files from anvil")

	timelineCmd.AddCommand(timelineListCmd)
	timelineCmd.AddCommand(timelineCreateCmd)
	timelineCmd.AddCommand(timelineSwitchCmd)
	
	portalCmd.AddCommand(portalAddCmd)
	portalCmd.AddCommand(portalListCmd)
	portalCmd.AddCommand(portalRemoveCmd)
	
	versionCmd.AddCommand(versionCreateCmd)
	versionCmd.AddCommand(versionListCmd)
	versionCmd.AddCommand(versionPushCmd)
	
	versionPushCmd.Flags().Bool("all", false, "Push all versions")
	
	syncCmd.Flags().Bool("push", false, "Only push changes")
	syncCmd.Flags().Bool("pull", false, "Only pull changes")
}