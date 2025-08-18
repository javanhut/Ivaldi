package cli

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

var rootCmd = &cobra.Command{
	Use:   "ivaldi",
	Short: "Ivaldi - A human-centered version control system",
	Long: `Ivaldi is a next-generation version control system that replaces Git
with intuitive commands and human-friendly terminology.

Named after the Norse dwarf craftsman, Ivaldi helps you craft
your code with precision and care.`,
	Run: func(cmd *cobra.Command, args []string) {
		cmd.Help()
	},
}

func Execute() error {
	return rootCmd.Execute()
}

func init() {
	rootCmd.AddCommand(forgeCmd)
	rootCmd.AddCommand(mirrorCmd)
	rootCmd.AddCommand(gatherCmd)
	rootCmd.AddCommand(discardCmd)
	rootCmd.AddCommand(sealCmd)
	rootCmd.AddCommand(timelineCmd)
	rootCmd.AddCommand(jumpCmd)
	rootCmd.AddCommand(fuseCmd)
	rootCmd.AddCommand(reshapeCmd)
	rootCmd.AddCommand(pluckCmd)
	rootCmd.AddCommand(shelfCmd)
	rootCmd.AddCommand(portalCmd)
	rootCmd.AddCommand(versionCmd)
	rootCmd.AddCommand(scoutCmd)
	rootCmd.AddCommand(syncCmd)
	rootCmd.AddCommand(statusCmd)
	rootCmd.AddCommand(logCmd)
}

func checkRepo() error {
	if _, err := os.Stat(".ivaldi"); os.IsNotExist(err) {
		return fmt.Errorf("not in an Ivaldi repository (no .ivaldi directory found)")
	}
	return nil
}