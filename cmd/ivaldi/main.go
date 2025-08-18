package main

import (
	"fmt"
	"os"

	"ivaldi/ui/enhanced_cli"
)

func main() {
	if err := enhanced_cli.Execute(); err != nil {
		fmt.Fprintf(os.Stderr, "Error: %v\n", err)
		os.Exit(1)
	}
}