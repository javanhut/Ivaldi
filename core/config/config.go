package config

import (
	"bufio"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"syscall"

	"golang.org/x/term"
)

// Credentials represents stored authentication credentials
type Credentials struct {
	GitHubToken string `json:"github_token,omitempty"`
	GitLabToken string `json:"gitlab_token,omitempty"`
	UserName    string `json:"user_name,omitempty"`
	UserEmail   string `json:"user_email,omitempty"`
}

// ConfigManager handles Ivaldi configuration
type ConfigManager struct {
	configPath string
}

// NewConfigManager creates a new config manager
func NewConfigManager(root string) *ConfigManager {
	configPath := filepath.Join(root, ".ivaldi", "config.json")
	return &ConfigManager{
		configPath: configPath,
	}
}

// LoadCredentials loads stored credentials
func (cm *ConfigManager) LoadCredentials() (*Credentials, error) {
	if _, err := os.Stat(cm.configPath); os.IsNotExist(err) {
		return &Credentials{}, nil
	}

	data, err := os.ReadFile(cm.configPath)
	if err != nil {
		return nil, err
	}

	var creds Credentials
	if err := json.Unmarshal(data, &creds); err != nil {
		return nil, err
	}

	return &creds, nil
}

// SaveCredentials saves credentials to config file
func (cm *ConfigManager) SaveCredentials(creds *Credentials) error {
	// Ensure config directory exists
	if err := os.MkdirAll(filepath.Dir(cm.configPath), 0755); err != nil {
		return err
	}

	data, err := json.MarshalIndent(creds, "", "  ")
	if err != nil {
		return err
	}

	return os.WriteFile(cm.configPath, data, 0600) // Secure permissions for credentials
}

// InteractiveSetup walks user through credential setup
func (cm *ConfigManager) InteractiveSetup() error {
	fmt.Println("=== Ivaldi Configuration Setup ===")
	fmt.Println("Configure your credentials for remote repositories")
	fmt.Println()

	creds, err := cm.LoadCredentials()
	if err != nil {
		creds = &Credentials{}
	}

	reader := bufio.NewReader(os.Stdin)

	// User info
	fmt.Printf("User name [%s]: ", creds.UserName)
	if name, _ := reader.ReadString('\n'); strings.TrimSpace(name) != "" {
		creds.UserName = strings.TrimSpace(name)
	}

	fmt.Printf("User email [%s]: ", creds.UserEmail)
	if email, _ := reader.ReadString('\n'); strings.TrimSpace(email) != "" {
		creds.UserEmail = strings.TrimSpace(email)
	}

	// GitHub token
	fmt.Println()
	fmt.Println("GitHub Personal Access Token:")
	fmt.Println("  1. Go to https://github.com/settings/tokens")
	fmt.Println("  2. Generate a new token with 'repo' permissions")
	fmt.Println("  3. Copy the token and paste it here")
	fmt.Printf("GitHub token [%s]: ", maskToken(creds.GitHubToken))
	
	token, err := cm.readSecureInput()
	if err != nil {
		return err
	}
	if strings.TrimSpace(token) != "" {
		creds.GitHubToken = strings.TrimSpace(token)
	}

	// Validate GitHub token if provided
	if creds.GitHubToken != "" {
		fmt.Print("Validating GitHub token... ")
		if err := cm.validateGitHubToken(creds.GitHubToken); err != nil {
			fmt.Printf("❌ Invalid: %v\n", err)
			return fmt.Errorf("GitHub token validation failed: %v", err)
		}
		fmt.Println("✅ Valid")
	}

	// GitLab token (optional)
	fmt.Println()
	fmt.Printf("GitLab token (optional) [%s]: ", maskToken(creds.GitLabToken))
	token, err = cm.readSecureInput()
	if err != nil {
		return err
	}
	if strings.TrimSpace(token) != "" {
		creds.GitLabToken = strings.TrimSpace(token)
	}

	// Validate GitLab token if provided
	if creds.GitLabToken != "" {
		fmt.Print("Validating GitLab token... ")
		if err := cm.validateGitLabToken(creds.GitLabToken); err != nil {
			fmt.Printf("❌ Invalid: %v\n", err)
			return fmt.Errorf("GitLab token validation failed: %v", err)
		}
		fmt.Println("✅ Valid")
	}

	// Save credentials
	if err := cm.SaveCredentials(creds); err != nil {
		return fmt.Errorf("failed to save credentials: %v", err)
	}

	fmt.Println()
	fmt.Println("✅ Configuration saved successfully!")
	fmt.Printf("Credentials stored in: %s\n", cm.configPath)
	fmt.Println("You can now use 'ivaldi upload' to push to remote repositories")

	return nil
}

// readSecureInput reads password/token input without echoing
func (cm *ConfigManager) readSecureInput() (string, error) {
	if !term.IsTerminal(int(syscall.Stdin)) {
		// Not a terminal, read normally
		reader := bufio.NewReader(os.Stdin)
		input, _ := reader.ReadString('\n')
		return strings.TrimSpace(input), nil
	}

	// Terminal input, hide characters
	bytePassword, err := term.ReadPassword(int(syscall.Stdin))
	if err != nil {
		return "", err
	}
	fmt.Println() // Add newline after hidden input
	return string(bytePassword), nil
}

// validateGitHubToken validates a GitHub personal access token
func (cm *ConfigManager) validateGitHubToken(token string) error {
	// Test the token by making a simple API call
	req, err := http.NewRequest("GET", "https://api.github.com/user", nil)
	if err != nil {
		return err
	}

	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")

	client := &http.Client{}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("network error: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode == 401 {
		return fmt.Errorf("invalid token")
	}
	if resp.StatusCode != 200 {
		return fmt.Errorf("API error: %s", resp.Status)
	}

	return nil
}

// validateGitLabToken validates a GitLab personal access token
func (cm *ConfigManager) validateGitLabToken(token string) error {
	// Test the token by making a simple API call
	req, err := http.NewRequest("GET", "https://gitlab.com/api/v4/user", nil)
	if err != nil {
		return err
	}

	req.Header.Set("Authorization", "Bearer "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")

	client := &http.Client{}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("network error: %v", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode == 401 {
		return fmt.Errorf("invalid token")
	}
	if resp.StatusCode != 200 {
		return fmt.Errorf("API error: %s", resp.Status)
	}

	return nil
}

// maskToken returns a masked version of a token for display
func maskToken(token string) string {
	if token == "" {
		return "not set"
	}
	if len(token) <= 8 {
		return "***"
	}
	return token[:4] + "***" + token[len(token)-4:]
}

// GetGitHubToken returns the stored GitHub token
func (cm *ConfigManager) GetGitHubToken() (string, error) {
	creds, err := cm.LoadCredentials()
	if err != nil {
		return "", err
	}
	return creds.GitHubToken, nil
}

// GetGitLabToken returns the stored GitLab token
func (cm *ConfigManager) GetGitLabToken() (string, error) {
	creds, err := cm.LoadCredentials()
	if err != nil {
		return "", err
	}
	return creds.GitLabToken, nil
}

// GetUserInfo returns the stored user information
func (cm *ConfigManager) GetUserInfo() (name, email string, err error) {
	creds, err := cm.LoadCredentials()
	if err != nil {
		return "", "", err
	}
	return creds.UserName, creds.UserEmail, nil
}