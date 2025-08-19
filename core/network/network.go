package network

import (
	"bytes"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"time"

	"ivaldi/core/config"
	"ivaldi/core/objects"
	"ivaldi/storage/local"
	"ivaldi/core/workspace"
)

// NetworkManager handles remote operations without git dependencies
type NetworkManager struct {
	client           *http.Client
	root             string // Repository root for loading config
	downloadProgress *downloadProgress
}

// downloadProgress tracks download progress
type downloadProgress struct {
	total      int
	downloaded int
	mutex      sync.Mutex
}

type downloadJob struct {
	url      string
	path     string
	content  string
	encoding string
}

type downloadWorker struct {
	id       int
	jobs     <-chan downloadJob
	results  chan<- error
	client   *http.Client
	progress *downloadProgress
}

// NewNetworkManager creates a new network manager
func NewNetworkManager(root string) *NetworkManager {
	return &NetworkManager{
		client: &http.Client{
			Timeout: 30 * time.Second,
		},
		root: root,
	}
}

// getGitHubToken gets GitHub token with multiple fallback options
func (nm *NetworkManager) getGitHubToken() (string, error) {
	// Try local config first
	configMgr := config.NewConfigManager(nm.root)
	return configMgr.GetGitHubTokenWithFallback()
}

// RemoteRef represents a reference on the remote
type RemoteRef struct {
	Name   string      `json:"name"`
	Hash   objects.Hash `json:"hash"`
	Type   string      `json:"type"` // "timeline" or "tag"
}

// FetchResult contains the result of a fetch operation
type FetchResult struct {
	Refs    []RemoteRef `json:"refs"`
	Seals   []*objects.Seal `json:"seals"`
	Objects []objects.Hash `json:"objects"`
}

// FetchFromPortal fetches changes from a remote portal using GitHub API
func (nm *NetworkManager) FetchFromPortal(portalURL, timeline string) (*FetchResult, error) {
	// Handle GitHub repositories
	if strings.Contains(portalURL, "github.com") {
		return nm.fetchFromGitHub(portalURL, timeline)
	} else if strings.Contains(portalURL, "gitlab.com") {
		return nm.fetchFromGitLab(portalURL, timeline)
	} else {
		// Native Ivaldi repository
		return nm.fetchFromIvaldiRepo(portalURL, timeline)
	}
}

// UploadToPortal uploads changes to a remote portal using Ivaldi's native approach
func (nm *NetworkManager) UploadToPortal(portalURL, timeline string, seals []*objects.Seal) error {
	// Determine the portal type and handle accordingly
	if strings.Contains(portalURL, "github.com") {
		return nm.uploadToGitHub(portalURL, timeline, seals)
	} else if strings.Contains(portalURL, "gitlab.com") {
		return nm.uploadToGitLab(portalURL, timeline, seals)
	} else {
		// Native Ivaldi repository
		return nm.uploadToIvaldiRepo(portalURL, timeline, seals)
	}
}

// DownloadIvaldiRepo downloads an Ivaldi repository from a remote URL
func (nm *NetworkManager) DownloadIvaldiRepo(url, dest string) error {
	// Use HTTP-based download for all repository types
	if strings.Contains(url, "github.com") {
		return nm.downloadFromGitHub(url, dest)
	} else if strings.Contains(url, "gitlab.com") {
		return nm.downloadFromGitLab(url, dest)
	} else {
		return nm.downloadFromIvaldiRepo(url, dest)
	}
}

// downloadFromGitHub downloads from GitHub using their API
func (nm *NetworkManager) downloadFromGitHub(url, dest string) error {
	// Extract owner and repo from URL
	urlParts := strings.Split(strings.TrimSuffix(url, ".git"), "/")
	if len(urlParts) < 2 {
		return fmt.Errorf("invalid GitHub URL format: %s", url)
	}
	
	owner := urlParts[len(urlParts)-2]
	repo := urlParts[len(urlParts)-1]
	
	fmt.Printf("Downloading repository: %s/%s\n", owner, repo)
	
	// Create destination directory
	if err := os.MkdirAll(dest, 0755); err != nil {
		return fmt.Errorf("failed to create directory: %v", err)
	}
	
	// First, count total files for progress bar
	fmt.Print("├─ Analyzing repository structure... ")
	totalFiles, err := nm.countGitHubFiles(owner, repo, "")
	if err != nil {
		fmt.Println("Failed")
		return fmt.Errorf("failed to analyze repository: %v", err)
	}
	fmt.Printf("Done (%d files)\n", totalFiles)
	
	// Use tarball download for large repositories (faster)
	if totalFiles > 100 {
		fmt.Printf("├─ Large repository detected (%d files), using optimized download...\n", totalFiles)
		return nm.downloadGitHubTarball(owner, repo, dest)
	}
	
	// Initialize download progress tracking
	nm.downloadProgress = &downloadProgress{
		total:      totalFiles,
		downloaded: 0,
		mutex:      sync.Mutex{},
	}
	
	// Use optimized parallel download
	err = nm.downloadGitHubContentsParallel(owner, repo, dest)
	if err != nil {
		fmt.Println("\n└─ Download failed")
		return err
	}
	
	fmt.Printf("└─ Successfully downloaded %d files\n", totalFiles)
	return nil
}

// downloadGitHubContents recursively downloads repository contents using GitHub API
func (nm *NetworkManager) downloadGitHubContents(owner, repo, localPath, remotePath string) error {
	// GitHub API endpoint for repository contents
	apiURL := fmt.Sprintf("https://api.github.com/repos/%s/%s/contents", owner, repo)
	if remotePath != "" {
		apiURL += "/" + remotePath
	}
	
	req, err := http.NewRequest("GET", apiURL, nil)
	if err != nil {
		return err
	}
	
	// Add authentication if available
	if token, err := nm.getGitHubToken(); err == nil && token != "" {
		req.Header.Set("Authorization", "token "+token)
	}
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 {
		body, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("GitHub API error (%d): %s", resp.StatusCode, string(body))
	}
	
	var contents []struct {
		Name        string `json:"name"`
		Path        string `json:"path"`
		Type        string `json:"type"`
		DownloadURL string `json:"download_url"`
		Content     string `json:"content"`
		Encoding    string `json:"encoding"`
	}
	
	if err := json.NewDecoder(resp.Body).Decode(&contents); err != nil {
		return err
	}
	
	// Process each item
	for _, item := range contents {
		localItemPath := filepath.Join(localPath, item.Name)
		
		if item.Type == "dir" {
			// Create directory and recurse
			if err := os.MkdirAll(localItemPath, 0755); err != nil {
				return err
			}
			if err := nm.downloadGitHubContents(owner, repo, localItemPath, item.Path); err != nil {
				return err
			}
		} else if item.Type == "file" {
			// Download file
			if item.DownloadURL != "" {
				// Download from download URL
				if err := nm.DownloadFile(item.DownloadURL, localItemPath); err != nil {
					return fmt.Errorf("failed to download %s: %v", item.Path, err)
				}
			} else if item.Content != "" && item.Encoding == "base64" {
				// Decode base64 content for small files
				content, err := base64.StdEncoding.DecodeString(item.Content)
				if err != nil {
					return fmt.Errorf("failed to decode %s: %v", item.Path, err)
				}
				if err := os.WriteFile(localItemPath, content, 0644); err != nil {
					return err
				}
			}
			// Update progress instead of printing each file
			if nm.downloadProgress != nil {
				nm.downloadProgress.downloaded++
				nm.showDownloadProgress()
			}
		}
	}
	
	return nil
}

// countGitHubFiles counts total files in repository for progress tracking
func (nm *NetworkManager) countGitHubFiles(owner, repo, remotePath string) (int, error) {
	// GitHub API endpoint for repository contents
	apiURL := fmt.Sprintf("https://api.github.com/repos/%s/%s/contents", owner, repo)
	if remotePath != "" {
		apiURL += "/" + remotePath
	}
	
	req, err := http.NewRequest("GET", apiURL, nil)
	if err != nil {
		return 0, err
	}
	
	// Add authentication if available
	if token, err := nm.getGitHubToken(); err == nil && token != "" {
		req.Header.Set("Authorization", "token "+token)
	}
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return 0, err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 {
		return 0, fmt.Errorf("GitHub API error: %d", resp.StatusCode)
	}
	
	var contents []struct {
		Name string `json:"name"`
		Path string `json:"path"`
		Type string `json:"type"`
	}
	
	if err := json.NewDecoder(resp.Body).Decode(&contents); err != nil {
		return 0, err
	}
	
	count := 0
	for _, item := range contents {
		if item.Type == "dir" {
			// Recursively count files in subdirectories
			subCount, err := nm.countGitHubFiles(owner, repo, item.Path)
			if err != nil {
				return 0, err
			}
			count += subCount
		} else if item.Type == "file" {
			count++
		}
	}
	
	return count, nil
}

// showDownloadProgress displays a visual progress bar
func (nm *NetworkManager) showDownloadProgress() {
	if nm.downloadProgress == nil {
		return
	}
	
	// Thread-safe progress reading
	nm.downloadProgress.mutex.Lock()
	downloaded := nm.downloadProgress.downloaded
	total := nm.downloadProgress.total
	nm.downloadProgress.mutex.Unlock()
	
	percentage := (downloaded * 100) / total
	barLength := 30
	filled := (barLength * downloaded) / total
	
	// Create progress bar
	bar := "["
	for i := 0; i < barLength; i++ {
		if i < filled {
			bar += "="
		} else if i == filled {
			bar += ">"
		} else {
			bar += " "
		}
	}
	bar += "]"
	
	// Clear line and print progress
	fmt.Printf("\r├─ Downloading files... %s %d%% (%d/%d)", bar, percentage, downloaded, total)
	
	// Add newline when complete
	if downloaded == total {
		fmt.Println()
	}
}

// updateProgress safely increments the download counter
func (nm *NetworkManager) updateProgress() {
	if nm.downloadProgress == nil {
		return
	}
	
	nm.downloadProgress.mutex.Lock()
	nm.downloadProgress.downloaded++
	nm.downloadProgress.mutex.Unlock()
	
	nm.showDownloadProgress()
}

// downloadWorker processes download jobs concurrently
func (dw *downloadWorker) start() {
	for job := range dw.jobs {
		var err error
		
		if job.url != "" {
			// Download from URL
			err = dw.downloadFromURL(job.url, job.path)
		} else if job.content != "" && job.encoding == "base64" {
			// Decode base64 content
			err = dw.decodeBase64ToFile(job.content, job.path)
		}
		
		if dw.progress != nil {
			dw.progress.mutex.Lock()
			dw.progress.downloaded++
			dw.progress.mutex.Unlock()
		}
		
		dw.results <- err
	}
}

// downloadFromURL downloads a file from URL
func (dw *downloadWorker) downloadFromURL(url, localPath string) error {
	resp, err := dw.client.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode != 200 {
		return fmt.Errorf("HTTP %d downloading %s", resp.StatusCode, url)
	}

	// Ensure directory exists
	if err := os.MkdirAll(filepath.Dir(localPath), 0755); err != nil {
		return err
	}

	file, err := os.Create(localPath)
	if err != nil {
		return err
	}
	defer file.Close()

	_, err = io.Copy(file, resp.Body)
	return err
}

// decodeBase64ToFile decodes base64 content to a file
func (dw *downloadWorker) decodeBase64ToFile(content, localPath string) error {
	decoded, err := base64.StdEncoding.DecodeString(content)
	if err != nil {
		return err
	}
	
	// Ensure directory exists
	if err := os.MkdirAll(filepath.Dir(localPath), 0755); err != nil {
		return err
	}
	
	return os.WriteFile(localPath, decoded, 0644)
}

// downloadGitHubContentsParallel downloads all files using parallel workers
func (nm *NetworkManager) downloadGitHubContentsParallel(owner, repo, localPath string) error {
	// First, collect all files to download  
	fmt.Print("├─ Collecting file list... ")
	fileJobs, err := nm.collectAllFiles(owner, repo, localPath, "")
	if err != nil {
		fmt.Println("Failed")
		return err
	}
	fmt.Printf("Done (%d files)\n", len(fileJobs))
	
	// Update total count if different
	if nm.downloadProgress != nil {
		nm.downloadProgress.mutex.Lock()
		nm.downloadProgress.total = len(fileJobs)
		nm.downloadProgress.downloaded = 0
		nm.downloadProgress.mutex.Unlock()
	}
	
	// Start parallel download workers
	const numWorkers = 8 // Configurable concurrency
	jobs := make(chan downloadJob, len(fileJobs))
	results := make(chan error, len(fileJobs))
	
	// Start workers
	for i := 0; i < numWorkers; i++ {
		worker := &downloadWorker{
			id:       i,
			jobs:     jobs,
			results:  results,
			client:   nm.client,
			progress: nm.downloadProgress,
		}
		go worker.start()
	}
	
	// Send jobs
	for _, job := range fileJobs {
		jobs <- job
	}
	close(jobs)
	
	// Collect results
	var firstError error
	for i := 0; i < len(fileJobs); i++ {
		if err := <-results; err != nil && firstError == nil {
			firstError = err
		}
		nm.showDownloadProgress()
	}
	
	return firstError
}

// collectAllFiles recursively collects all files to download
func (nm *NetworkManager) collectAllFiles(owner, repo, localPath, remotePath string) ([]downloadJob, error) {
	var jobs []downloadJob
	
	// GitHub API endpoint for repository contents
	apiURL := fmt.Sprintf("https://api.github.com/repos/%s/%s/contents", owner, repo)
	if remotePath != "" {
		apiURL += "/" + remotePath
	}
	
	req, err := http.NewRequest("GET", apiURL, nil)
	if err != nil {
		return nil, err
	}
	
	// Add authentication if available
	if token, err := nm.getGitHubToken(); err == nil && token != "" {
		req.Header.Set("Authorization", "token "+token)
	}
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 {
		body, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("GitHub API error (%d): %s", resp.StatusCode, string(body))
	}
	
	var contents []struct {
		Name        string `json:"name"`
		Path        string `json:"path"`
		Type        string `json:"type"`
		DownloadURL string `json:"download_url"`
		Content     string `json:"content"`
		Encoding    string `json:"encoding"`
	}
	
	if err := json.NewDecoder(resp.Body).Decode(&contents); err != nil {
		return nil, err
	}
	
	// Process each item
	for _, item := range contents {
		localItemPath := filepath.Join(localPath, item.Name)
		
		if item.Type == "dir" {
			// Create directory
			if err := os.MkdirAll(localItemPath, 0755); err != nil {
				return nil, err
			}
			// Recurse into subdirectory
			subJobs, err := nm.collectAllFiles(owner, repo, localItemPath, item.Path)
			if err != nil {
				return nil, err
			}
			jobs = append(jobs, subJobs...)
		} else if item.Type == "file" {
			// Add file to download jobs
			job := downloadJob{
				path:     localItemPath,
				url:      item.DownloadURL,
				content:  item.Content,
				encoding: item.Encoding,
			}
			jobs = append(jobs, job)
		}
	}
	
	return jobs, nil
}

// downloadGitHubTarball downloads repository as a tarball for better performance on large repos
func (nm *NetworkManager) downloadGitHubTarball(owner, repo, dest string) error {
	// GitHub tarball endpoint
	tarballURL := fmt.Sprintf("https://api.github.com/repos/%s/%s/tarball", owner, repo)
	
	req, err := http.NewRequest("GET", tarballURL, nil)
	if err != nil {
		return err
	}
	
	// Add authentication if available
	if token, err := nm.getGitHubToken(); err == nil && token != "" {
		req.Header.Set("Authorization", "token "+token)
	}
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	fmt.Print("├─ Downloading repository archive... ")
	resp, err := nm.client.Do(req)
	if err != nil {
		fmt.Println("Failed")
		return err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 {
		fmt.Println("Failed")
		body, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("GitHub API error (%d): %s", resp.StatusCode, string(body))
	}
	
	// Create a temporary file for the tarball
	tmpFile, err := os.CreateTemp("", "ivaldi-repo-*.tar.gz")
	if err != nil {
		fmt.Println("Failed")
		return err
	}
	defer os.Remove(tmpFile.Name())
	defer tmpFile.Close()
	
	// Download the tarball
	_, err = io.Copy(tmpFile, resp.Body)
	if err != nil {
		fmt.Println("Failed")
		return err
	}
	fmt.Println("Done")
	
	// Extract the tarball
	fmt.Print("├─ Extracting files... ")
	err = nm.extractTarball(tmpFile.Name(), dest)
	if err != nil {
		fmt.Println("Failed")
		return err
	}
	fmt.Println("Done")
	
	fmt.Printf("└─ Successfully downloaded repository using optimized method\n")
	return nil
}

// extractTarball extracts a gzipped tarball to destination
func (nm *NetworkManager) extractTarball(tarballPath, dest string) error {
	// Use tar command for extraction (faster and more reliable)
	cmd := exec.Command("tar", "-xzf", tarballPath, "-C", dest, "--strip-components=1")
	return cmd.Run()
}

// downloadFromGitLab downloads from GitLab using their API  
func (nm *NetworkManager) downloadFromGitLab(url, dest string) error {
	// Use GitLab API to download repository contents
	fmt.Printf("Would download GitLab repo %s to %s using API\n", url, dest)
	return nil // Placeholder
}

// downloadFromIvaldiRepo downloads from native Ivaldi repository
func (nm *NetworkManager) downloadFromIvaldiRepo(url, dest string) error {
	// Use native Ivaldi protocol to download repository
	fmt.Printf("Would download Ivaldi repo %s to %s using native protocol\n", url, dest)
	return nil // Placeholder
}

// CreatePortalConfig creates a portal configuration for a URL
func (nm *NetworkManager) CreatePortalConfig(name, url string) (*PortalConfig, error) {
	return &PortalConfig{
		Name: name,
		URL:  url,
		Type: nm.getPortalType(url),
	}, nil
}

// PortalConfig represents portal configuration
type PortalConfig struct {
	Name string `json:"name"`
	URL  string `json:"url"`
	Type string `json:"type"` // "ivaldi", "git", "github"
}

// getPortalType determines the portal type from URL
func (nm *NetworkManager) getPortalType(url string) string {
	if strings.Contains(url, "github.com") {
		return "github"
	}
	if strings.Contains(url, "gitlab.com") {
		return "gitlab"
	}
	if strings.HasSuffix(url, ".git") {
		return "git"
	}
	return "ivaldi"
}

// GitHubCommit represents a commit in GitHub API format
type GitHubCommit struct {
	Message string `json:"message"`
	Tree    string `json:"tree"`
	Parents []string `json:"parents"`
}

// GitHubTree represents a tree object for GitHub API
type GitHubTree struct {
	Tree []GitHubTreeItem `json:"tree"`
}

// GitHubTreeItem represents a file in a GitHub tree
type GitHubTreeItem struct {
	Path    string `json:"path"`
	Mode    string `json:"mode"`
	Type    string `json:"type"`
	Content string `json:"content,omitempty"`
	SHA     string `json:"sha,omitempty"`
}

// uploadToGitHub uploads changes to GitHub using their API
func (nm *NetworkManager) uploadToGitHub(portalURL, timeline string, seals []*objects.Seal) error {
	// Extract owner and repo from URL
	// e.g., https://github.com/javanhut/Ivaldi.git -> javanhut, Ivaldi
	urlParts := strings.Split(strings.TrimSuffix(portalURL, ".git"), "/")
	if len(urlParts) < 2 {
		return fmt.Errorf("invalid GitHub URL format: %s", portalURL)
	}
	
	owner := urlParts[len(urlParts)-2]
	repo := urlParts[len(urlParts)-1]
	
	if len(seals) == 0 {
		return fmt.Errorf("no seals to upload")
	}
	
	fmt.Printf("Uploading to GitHub repo: %s/%s\n", owner, repo)
	
	// Get files that have changed and need to be uploaded
	changedFiles, allFiles, isFirstUpload, err := nm.getFilesForUpload(owner, repo, seals)
	if err != nil {
		return fmt.Errorf("failed to get files for upload: %v", err)
	}
	
	if len(changedFiles) == 0 {
		fmt.Println("Repository is already up-to-date with remote")
		return nil
	}
	
	// Show what we're about to upload
	action := "Uploading"
	if isFirstUpload {
		action = "Initial upload -"
	} else {
		fmt.Printf("Changed: %d files (of %d total)\n", len(changedFiles), len(allFiles))
	}
	fmt.Printf("%s %d files to GitHub repo: %s/%s\n", action, len(allFiles), owner, repo)
	
	latestSeal := seals[len(seals)-1]
	err = nm.uploadCompleteRepositoryState(owner, repo, timeline, latestSeal, allFiles, changedFiles)
	if err != nil {
		return err
	}
	
	// Save upload state after successful upload
	return nm.saveUploadStateAfterUpload(owner, repo, latestSeal, allFiles)
}

// uploadToGitLab uploads changes to GitLab using their API
func (nm *NetworkManager) uploadToGitLab(portalURL, timeline string, seals []*objects.Seal) error {
	// Similar to GitHub but using GitLab API
	fmt.Printf("Would upload %d seals to GitLab repo via API\n", len(seals))
	return nil // Placeholder success
}

// uploadToIvaldiRepo uploads changes to a native Ivaldi repository
func (nm *NetworkManager) uploadToIvaldiRepo(portalURL, timeline string, seals []*objects.Seal) error {
	// Use native Ivaldi protocol for repository-to-repository communication
	data := map[string]interface{}{
		"timeline": timeline,
		"seals":    seals,
	}
	
	jsonData, err := json.Marshal(data)
	if err != nil {
		return err
	}
	
	// Send POST request to Ivaldi repository endpoint
	resp, err := nm.client.Post(portalURL+"/api/upload", "application/json", bytes.NewBuffer(jsonData))
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("upload failed: %s", resp.Status)
	}
	
	return nil
}

// IsGitRepo checks if the URL points to a git repository (for backwards compatibility)
func (nm *NetworkManager) IsGitRepo(url string) bool {
	return strings.Contains(url, "github.com") || strings.Contains(url, "gitlab.com") || strings.HasSuffix(url, ".git")
}

// createOrUpdateFile creates or updates a file in a GitHub repository
func (nm *NetworkManager) createOrUpdateFile(owner, repo, path, content, message string) error {
	// GitHub API endpoint for file operations
	apiURL := fmt.Sprintf("https://api.github.com/repos/%s/%s/contents/%s", owner, repo, path)
	
	// First, try to get the existing file to get its SHA
	var existingSHA string
	req, err := http.NewRequest("GET", apiURL, nil)
	if err != nil {
		return err
	}
	
	// Add authentication for GET request too
	token, err := nm.getGitHubToken()
	if err != nil {
		return fmt.Errorf("failed to load GitHub token: %v", err)
	}
	if token == "" {
		return fmt.Errorf("GitHub token not configured")
	}
	
	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	resp, err := nm.client.Do(req)
	if err == nil && resp.StatusCode == 200 {
		defer resp.Body.Close()
		var fileInfo struct {
			SHA string `json:"sha"`
		}
		if json.NewDecoder(resp.Body).Decode(&fileInfo) == nil {
			existingSHA = fileInfo.SHA
		}
	}
	
	// Prepare the request body (GitHub API requires base64 encoded content)
	encodedContent := base64.StdEncoding.EncodeToString([]byte(content))
	requestBody := map[string]interface{}{
		"message": message,
		"content": encodedContent,
	}
	
	// If file exists, include the SHA for update
	if existingSHA != "" {
		requestBody["sha"] = existingSHA
	}
	
	jsonData, err := json.Marshal(requestBody)
	if err != nil {
		return err
	}
	
	// Create PUT request
	putReq, err := http.NewRequest("PUT", apiURL, bytes.NewBuffer(jsonData))
	if err != nil {
		return err
	}
	
	putReq.Header.Set("Content-Type", "application/json")
	putReq.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	// Load and add authentication
	token2, err := nm.getGitHubToken()
	if err != nil {
		return fmt.Errorf("failed to load GitHub token: %v", err)
	}
	if token2 == "" {
		return fmt.Errorf("GitHub token not configured. Run 'ivaldi config' to set up authentication")
	}
	
	putReq.Header.Set("Authorization", "token "+token2)
	
	// Send request
	resp, err = nm.client.Do(putReq)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 && resp.StatusCode != 201 {
		body, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("GitHub API error (%d): %s", resp.StatusCode, string(body))
	}
	
	return nil
}

// uploadCompleteRepositoryState uploads complete repo state but only reads changed files for efficiency
func (nm *NetworkManager) uploadCompleteRepositoryState(owner, repo, timeline string, seal *objects.Seal, allFiles, changedFiles map[string]string) error {
	// Load ignore patterns
	ignorePatterns, err := nm.loadIgnorePatterns()
	if err != nil {
		fmt.Printf("Warning: failed to load ignore patterns: %v\n", err)
		ignorePatterns = []string{} // Continue without ignore patterns
	}
	
	// Prepare ALL repository files for upload (complete state)
	var filesToUpload []FileToUpload
	
	for relPath := range allFiles {
		// Convert to forward slashes for consistent matching
		cleanPath := strings.ReplaceAll(relPath, "\\", "/")
		
		// Check if file should be ignored
		if nm.shouldIgnoreFile(cleanPath, ignorePatterns) {
			fmt.Printf("Skipping ignored file: %s\n", cleanPath)
			continue
		}
		
		// Get full path
		fullPath := filepath.Join(nm.root, relPath)
		
		// Check if file exists (might have been deleted)
		if _, err := os.Stat(fullPath); os.IsNotExist(err) {
			// File was deleted - for now we'll skip it in uploads since GitHub
			// API handles this differently. TODO: implement proper deletion handling
			fmt.Printf("Skipping deleted file: %s\n", cleanPath)
			continue
		}
		
		// Read file content
		content, err := os.ReadFile(fullPath)
		if err != nil {
			fmt.Printf("Warning: failed to read %s: %v\n", cleanPath, err)
			continue
		}
		
		filesToUpload = append(filesToUpload, FileToUpload{
			Path:    cleanPath,
			Content: content,
		})
	}
	
	if len(filesToUpload) == 0 {
		fmt.Println("No files to upload")
		return nil
	}
	
	fmt.Printf("Repository state: %d total files, %d changed\n", len(allFiles), len(changedFiles))
	
	// Upload complete repository state with progress bar
	return nm.uploadFilesBatchWithProgress(owner, repo, timeline, filesToUpload, seal)
}

type FileToUpload struct {
	Path    string
	Content []byte
}

// loadIgnorePatterns loads patterns from .ivaldiignore file
func (nm *NetworkManager) loadIgnorePatterns() ([]string, error) {
	ignoreFile := filepath.Join(nm.root, ".ivaldiignore")
	content, err := os.ReadFile(ignoreFile)
	if err != nil {
		if os.IsNotExist(err) {
			return []string{}, nil // No ignore file is fine
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

// shouldIgnoreFile checks if a file path matches any ignore pattern
func (nm *NetworkManager) shouldIgnoreFile(filePath string, patterns []string) bool {
	// Always ignore these directories
	builtInIgnores := []string{
		".ivaldi/",
		".git/",
		"build/",
		"*.tmp",
		"*.temp",
		"*~",
		".DS_Store",
		"*.log",
		"*.bak",
	}
	
	allPatterns := append(patterns, builtInIgnores...)
	
	for _, pattern := range allPatterns {
		if nm.matchesPattern(filePath, pattern) {
			return true
		}
	}
	
	return false
}

// matchesPattern checks if a file path matches a glob-like pattern
func (nm *NetworkManager) matchesPattern(filePath, pattern string) bool {
	// Simple pattern matching - convert glob to regex
	pattern = strings.ReplaceAll(pattern, ".", "\\.")
	pattern = strings.ReplaceAll(pattern, "*", ".*")
	pattern = strings.ReplaceAll(pattern, "?", ".")
	
	// Handle directory patterns
	if strings.HasSuffix(pattern, "/") {
		pattern = pattern + ".*"
	}
	
	// Add anchors
	pattern = "^" + pattern + "$"
	
	matched, err := regexp.MatchString(pattern, filePath)
	if err != nil {
		// If regex is invalid, fall back to simple string matching
		return strings.Contains(filePath, strings.ReplaceAll(pattern, ".*", ""))
	}
	
	return matched
}

// uploadFilesBatchWithProgress uploads files with a progress bar - much more efficient than git
func (nm *NetworkManager) uploadFilesBatchWithProgress(owner, repo, timeline string, files []FileToUpload, seal *objects.Seal) error {
	if len(files) == 0 {
		return nil
	}

	// Progress bar setup
	fmt.Printf("Uploading %d files (batch operation)...\n", len(files))
	
	// Step 1: Get current commit (with progress)
	fmt.Print("├─ Getting remote state... ")
	currentSHA, err := nm.getCurrentCommitSHA(owner, repo, timeline)
	if err != nil {
		fmt.Printf("Creating new branch: %s\n", timeline)
		// For new branches, inherit from main branch to maintain history
		mainSHA, mainErr := nm.getCurrentCommitSHA(owner, repo, "main")
		if mainErr != nil {
			// If even main doesn't exist, this is truly a new repository
			currentSHA = ""
		} else {
			currentSHA = mainSHA
			fmt.Printf("├─ Branching from main... Done\n")
		}
	} else {
		fmt.Println("Done")
	}
	
	// Step 2: Prepare tree data (with progress)  
	fmt.Printf("├─ Preparing %d files... ", len(files))
	treeItems := make([]GitHubTreeItem, 0, len(files))
	for _, file := range files {
		treeItems = append(treeItems, GitHubTreeItem{
			Path:    file.Path,
			Mode:    "100644", // regular file
			Type:    "blob", 
			Content: string(file.Content),
		})
	}
	
	tree := GitHubTree{Tree: treeItems}
	treeData, err := json.Marshal(tree)
	if err != nil {
		fmt.Println("Failed")
		return fmt.Errorf("failed to prepare files: %v", err)
	}
	fmt.Println("Done")
	
	// Step 3: Create tree (batch operation)
	fmt.Print("├─ Creating tree object... ")
	treeSHA, err := nm.createTree(owner, repo, treeData)
	if err != nil {
		fmt.Println("Failed")
		return fmt.Errorf("failed to create tree: %v", err)
	}
	fmt.Println("Done")
	
	// Step 4: Create commit
	fmt.Print("├─ Creating commit... ")
	parents := []string{} // Initialize as empty slice, not nil
	if currentSHA != "" {
		parents = []string{currentSHA}
	}
	
	commit := GitHubCommit{
		Message: fmt.Sprintf("%s\n\nSealed as: %s", seal.Message, seal.Name),
		Tree:    treeSHA,
		Parents: parents,
	}
	
	commitSHA, err := nm.createCommit(owner, repo, commit)
	if err != nil {
		fmt.Println("Failed")
		return fmt.Errorf("failed to create commit: %v", err)
	}
	fmt.Println("Done")
	
	// Step 5: Update or create branch
	fmt.Printf("└─ Setting branch: %s... ", timeline)
	err = nm.createOrUpdateReference(owner, repo, "heads/"+timeline, commitSHA)
	if err != nil {
		fmt.Println("Failed")
		return fmt.Errorf("failed to set branch: %v", err)
	}
	fmt.Println("Done")
	
	// Success message
	fmt.Printf("Successfully uploaded %d files in single atomic operation\n", len(files))
	fmt.Printf("Commit: %s\n", commitSHA[:12])
	
	return nil
}

// Legacy function kept for compatibility  
func (nm *NetworkManager) uploadFilesBatch(owner, repo string, files []FileToUpload, seal *objects.Seal) error {
	return nm.uploadFilesBatchWithProgress(owner, repo, "main", files, seal)
}

// UploadState tracks what was last uploaded to avoid redundant uploads
type UploadState struct {
	LastUploadedSeal string            `json:"last_uploaded_seal"`
	LastUploadTime   time.Time         `json:"last_upload_time"`
	FileHashes       map[string]string `json:"file_hashes"` // path -> hash
}

// getFilesForUpload returns changed files and all files for smart incremental upload
func (nm *NetworkManager) getFilesForUpload(owner, repo string, seals []*objects.Seal) (map[string]string, map[string]string, bool, error) {
	store, err := local.NewStore(nm.root, objects.BLAKE3)
	if err != nil {
		return nil, nil, false, fmt.Errorf("failed to create store: %v", err)
	}
	ws := workspace.New(nm.root, store)
	if err := ws.Scan(); err != nil {
		return nil, nil, false, fmt.Errorf("failed to scan workspace: %v", err)
	}

	// Get all current files
	allFiles := make(map[string]string)
	for path, fileState := range ws.Files {
		if fileState.Status != workspace.StatusDeleted {
			allFiles[path] = "tracked"
		}
	}

	// Load previous upload state
	uploadState, err := nm.loadUploadState(owner, repo)
	if err != nil {
		// First upload - all files are "changed"
		return allFiles, allFiles, true, nil
	}

	latestSeal := seals[len(seals)-1]
	// If we've already uploaded this seal, no changes needed
	if uploadState.LastUploadedSeal == latestSeal.Name {
		return map[string]string{}, allFiles, false, nil
	}

	// Find files that have actually changed
	changedFiles := make(map[string]string)

	// Check all current files for changes
	for path, fileState := range ws.Files {
		if fileState.Status == workspace.StatusDeleted {
			// File was deleted - need to remove from remote
			if _, wasUploaded := uploadState.FileHashes[path]; wasUploaded {
				changedFiles[path] = "deleted"
			}
			continue
		}

		// Calculate current hash
		currentHash := fileState.Hash.String()

		// Compare with last uploaded hash
		if lastHash, exists := uploadState.FileHashes[path]; !exists || lastHash != currentHash {
			changedFiles[path] = "modified"
		}
	}

	// Check for files that were uploaded before but no longer exist locally
	for path := range uploadState.FileHashes {
		if _, exists := allFiles[path]; !exists {
			changedFiles[path] = "deleted"
		}
	}

	return changedFiles, allFiles, false, nil
}

// loadUploadState loads the last upload state for a repository
func (nm *NetworkManager) loadUploadState(owner, repo string) (*UploadState, error) {
	statePath := filepath.Join(nm.root, ".ivaldi", "upload_state", fmt.Sprintf("%s_%s.json", owner, repo))
	
	data, err := os.ReadFile(statePath)
	if err != nil {
		return nil, err
	}
	
	var state UploadState
	if err := json.Unmarshal(data, &state); err != nil {
		return nil, err
	}
	
	return &state, nil
}

// saveUploadState saves the upload state after successful upload
func (nm *NetworkManager) saveUploadState(owner, repo string, state *UploadState) error {
	stateDir := filepath.Join(nm.root, ".ivaldi", "upload_state")
	if err := os.MkdirAll(stateDir, 0755); err != nil {
		return err
	}
	
	statePath := filepath.Join(stateDir, fmt.Sprintf("%s_%s.json", owner, repo))
	
	data, err := json.Marshal(state)
	if err != nil {
		return err
	}
	
	return os.WriteFile(statePath, data, 0644)
}

// saveUploadStateAfterUpload creates and saves upload state after successful upload
func (nm *NetworkManager) saveUploadStateAfterUpload(owner, repo string, seal *objects.Seal, allFiles map[string]string) error {
	store, err := local.NewStore(nm.root, objects.BLAKE3)
	if err != nil {
		return fmt.Errorf("failed to create store: %v", err)
	}
	ws := workspace.New(nm.root, store)
	if err := ws.Scan(); err != nil {
		return err
	}
	
	// Create new upload state
	fileHashes := make(map[string]string)
	for path, fileState := range ws.Files {
		if fileState.Status != workspace.StatusDeleted {
			fileHashes[path] = fileState.Hash.String()
		}
	}
	
	state := &UploadState{
		LastUploadedSeal: seal.Name,
		LastUploadTime:   time.Now(),
		FileHashes:       fileHashes,
	}
	
	return nm.saveUploadState(owner, repo, state)
}

// getAllRepositoryFiles loads the workspace and returns ALL repository files 
// This is used for initial uploads to ensure the remote has the complete repository state
func (nm *NetworkManager) getAllRepositoryFiles() (map[string]string, error) {
	store, err := local.NewStore(nm.root, objects.BLAKE3)
	if err != nil {
		return nil, fmt.Errorf("failed to create store: %v", err)
	}
	ws := workspace.New(nm.root, store)
	if err := ws.Scan(); err != nil {
		return nil, fmt.Errorf("failed to scan workspace: %v", err)
	}
	
	allFiles := make(map[string]string)
	
	// Include all files that are tracked by the workspace
	// This ensures we upload the complete repository state
	for path, fileState := range ws.Files {
		if fileState.Status != workspace.StatusDeleted {
			allFiles[path] = "tracked"
		}
	}
	
	fmt.Printf("Found %d repository files to upload\n", len(allFiles))
	return allFiles, nil
}

// getChangedFiles loads the workspace and returns only files that are staged for commit
func (nm *NetworkManager) getChangedFiles() (map[string]string, error) {
	store, err := local.NewStore(nm.root, objects.BLAKE3)
	if err != nil {
		return nil, fmt.Errorf("failed to create store: %v", err)
	}
	ws := workspace.New(nm.root, store)
	if err := ws.Scan(); err != nil {
		return nil, fmt.Errorf("failed to scan workspace: %v", err)
	}
	
	// Load the workspace state for the current timeline
	// Note: We need to know the current timeline to load the right state
	// For now, let's assume "main" - in a real implementation, this should be passed
	if err := ws.LoadState("main"); err != nil {
		fmt.Printf("Warning: failed to load workspace state: %v\n", err)
		// Continue without loaded state
	}
	
	changedFiles := make(map[string]string)
	
	// Only include files that are actually on the anvil (staged)
	// This ensures we only upload files that the user explicitly staged
	for path := range ws.AnvilFiles {
		changedFiles[path] = "staged"
	}
	
	fmt.Printf("Found %d staged files to upload\n", len(changedFiles))
	if len(changedFiles) > 0 {
		fmt.Println("Staged files:")
		for path := range changedFiles {
			fmt.Printf("  %s\n", path)
		}
	}
	
	return changedFiles, nil
}

// DownloadFile downloads a file from a URL
func (nm *NetworkManager) DownloadFile(url, dest string) error {
	resp, err := nm.client.Get(url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("failed to download: %s", resp.Status)
	}

	// Create destination directory if needed
	if err := os.MkdirAll(filepath.Dir(dest), 0755); err != nil {
		return err
	}

	file, err := os.Create(dest)
	if err != nil {
		return err
	}
	defer file.Close()

	_, err = io.Copy(file, resp.Body)
	return err
}

// getCurrentCommitSHA gets the current commit SHA for a branch
func (nm *NetworkManager) getCurrentCommitSHA(owner, repo, branch string) (string, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/%s/git/refs/heads/%s", owner, repo, branch)
	
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return "", err
	}
	
	// Add authentication
	token, err := nm.getGitHubToken()
	if err != nil {
		return "", err
	}
	
	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 {
		return "", fmt.Errorf("failed to get current commit: %s", resp.Status)
	}
	
	var ref struct {
		Object struct {
			SHA string `json:"sha"`
		} `json:"object"`
	}
	
	if err := json.NewDecoder(resp.Body).Decode(&ref); err != nil {
		return "", err
	}
	
	return ref.Object.SHA, nil
}

// createTree creates a tree object via GitHub API
func (nm *NetworkManager) createTree(owner, repo string, treeData []byte) (string, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/%s/git/trees", owner, repo)
	
	req, err := http.NewRequest("POST", url, bytes.NewBuffer(treeData))
	if err != nil {
		return "", err
	}
	
	// Add authentication
	token, err := nm.getGitHubToken()
	if err != nil {
		return "", err
	}
	
	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	req.Header.Set("Content-Type", "application/json")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 201 {
		body, _ := io.ReadAll(resp.Body)
		return "", fmt.Errorf("failed to create tree: %s - %s", resp.Status, string(body))
	}
	
	var tree struct {
		SHA string `json:"sha"`
	}
	
	if err := json.NewDecoder(resp.Body).Decode(&tree); err != nil {
		return "", err
	}
	
	return tree.SHA, nil
}

// createCommit creates a commit object via GitHub API
func (nm *NetworkManager) createCommit(owner, repo string, commit GitHubCommit) (string, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/%s/git/commits", owner, repo)
	
	commitData, err := json.Marshal(commit)
	if err != nil {
		return "", err
	}
	
	req, err := http.NewRequest("POST", url, bytes.NewBuffer(commitData))
	if err != nil {
		return "", err
	}
	
	// Add authentication
	token, err := nm.getGitHubToken()
	if err != nil {
		return "", err
	}
	
	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	req.Header.Set("Content-Type", "application/json")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return "", err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 201 {
		body, _ := io.ReadAll(resp.Body)
		return "", fmt.Errorf("failed to create commit: %s - %s", resp.Status, string(body))
	}
	
	var commitResp struct {
		SHA string `json:"sha"`
	}
	
	if err := json.NewDecoder(resp.Body).Decode(&commitResp); err != nil {
		return "", err
	}
	
	return commitResp.SHA, nil
}

// updateReference updates a branch reference via GitHub API
func (nm *NetworkManager) updateReference(owner, repo, ref, sha string) error {
	url := fmt.Sprintf("https://api.github.com/repos/%s/%s/git/refs/%s", owner, repo, ref)
	
	updateData := map[string]interface{}{
		"sha": sha,
	}
	
	jsonData, err := json.Marshal(updateData)
	if err != nil {
		return err
	}
	
	req, err := http.NewRequest("PATCH", url, bytes.NewBuffer(jsonData))
	if err != nil {
		return err
	}
	
	// Add authentication
	token, err := nm.getGitHubToken()
	if err != nil {
		return err
	}
	
	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	req.Header.Set("Content-Type", "application/json")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 {
		body, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("failed to update reference: %s - %s", resp.Status, string(body))
	}
	
	return nil
}

// createOrUpdateReference creates a new reference or updates existing one via GitHub API
func (nm *NetworkManager) createOrUpdateReference(owner, repo, ref, sha string) error {
	// First try to update existing reference
	err := nm.updateReference(owner, repo, ref, sha)
	if err != nil {
		// If update failed because reference doesn't exist, create it
		if strings.Contains(err.Error(), "Reference does not exist") {
			return nm.createReference(owner, repo, ref, sha)
		}
		return err
	}
	return nil
}

// createReference creates a new reference via GitHub API
func (nm *NetworkManager) createReference(owner, repo, ref, sha string) error {
	url := fmt.Sprintf("https://api.github.com/repos/%s/%s/git/refs", owner, repo)
	
	createData := map[string]interface{}{
		"ref": "refs/" + ref,
		"sha": sha,
	}
	
	jsonData, err := json.Marshal(createData)
	if err != nil {
		return err
	}
	
	req, err := http.NewRequest("POST", url, bytes.NewBuffer(jsonData))
	if err != nil {
		return err
	}
	
	// Add authentication
	token, err := nm.getGitHubToken()
	if err != nil {
		return err
	}
	
	req.Header.Set("Authorization", "token "+token)
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	req.Header.Set("Content-Type", "application/json")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 201 {
		body, _ := io.ReadAll(resp.Body)
		return fmt.Errorf("failed to create reference: %s - %s", resp.Status, string(body))
	}
	
	return nil
}

// GitHubCommitInfo represents commit information from GitHub API
type GitHubCommitInfo struct {
	Message string `json:"message"`
	Author  struct {
		Name  string    `json:"name"`
		Email string    `json:"email"`
		Date  time.Time `json:"date"`
	} `json:"author"`
	Committer struct {
		Name  string    `json:"name"`
		Email string    `json:"email"`
		Date  time.Time `json:"date"`
	} `json:"committer"`
	Tree struct {
		SHA string `json:"sha"`
	} `json:"tree"`
	Parents []struct {
		SHA string `json:"sha"`
	} `json:"parents"`
}

// fetchFromGitHub fetches changes from a GitHub repository
func (nm *NetworkManager) fetchFromGitHub(portalURL, timeline string) (*FetchResult, error) {
	// Extract owner and repo from URL
	urlParts := strings.Split(strings.TrimSuffix(portalURL, ".git"), "/")
	if len(urlParts) < 2 {
		return nil, fmt.Errorf("invalid GitHub URL format: %s", portalURL)
	}
	
	owner := urlParts[len(urlParts)-2]
	repo := urlParts[len(urlParts)-1]
	
	// Get the latest commit SHA for the timeline/branch
	commitSHA, err := nm.getCurrentCommitSHA(owner, repo, timeline)
	if err != nil {
		// Branch doesn't exist on remote
		return &FetchResult{
			Refs:    []RemoteRef{},
			Seals:   []*objects.Seal{},
			Objects: []objects.Hash{},
		}, nil
	}
	
	// Get commit information
	commit, err := nm.getGitHubCommit(owner, repo, commitSHA)
	if err != nil {
		return nil, fmt.Errorf("failed to get commit info: %v", err)
	}
	
	// Convert GitHub commit to Ivaldi seal
	seal := &objects.Seal{
		Name:      nm.generateMemorableNameFromCommit(commit),
		Iteration: 1, // TODO: Calculate proper iteration
		Message:   commit.Message,
		Author: objects.Identity{
			Name:  commit.Author.Name,
			Email: commit.Author.Email,
		},
		Timestamp: commit.Author.Date,
		Parents:   []objects.Hash{}, // TODO: Convert parent SHAs
	}
	
	// Let the storage layer calculate the hash when it stores the seal
	// For now, compute a temporary hash for the ref
	data, err := json.Marshal(seal)
	if err != nil {
		return nil, fmt.Errorf("failed to marshal seal: %v", err)
	}
	tempHash := objects.NewHash(data)
	
	// Create remote ref
	remoteRef := RemoteRef{
		Name: timeline,
		Hash: tempHash,
		Type: "timeline",
	}
	
	return &FetchResult{
		Refs:    []RemoteRef{remoteRef},
		Seals:   []*objects.Seal{seal},
		Objects: []objects.Hash{tempHash},
	}, nil
}

// getGitHubCommit gets commit information from GitHub API
func (nm *NetworkManager) getGitHubCommit(owner, repo, commitSHA string) (*GitHubCommitInfo, error) {
	url := fmt.Sprintf("https://api.github.com/repos/%s/%s/git/commits/%s", owner, repo, commitSHA)
	
	req, err := http.NewRequest("GET", url, nil)
	if err != nil {
		return nil, err
	}
	
	// Add authentication
	if token, err := nm.getGitHubToken(); err == nil && token != "" {
		req.Header.Set("Authorization", "token "+token)
	}
	req.Header.Set("User-Agent", "Ivaldi-VCS/1.0")
	
	resp, err := nm.client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	
	if resp.StatusCode != 200 {
		body, _ := io.ReadAll(resp.Body)
		return nil, fmt.Errorf("GitHub API error (%d): %s", resp.StatusCode, string(body))
	}
	
	var commit GitHubCommitInfo
	if err := json.NewDecoder(resp.Body).Decode(&commit); err != nil {
		return nil, err
	}
	
	return &commit, nil
}

// generateMemorableNameFromCommit generates a memorable name from a commit
func (nm *NetworkManager) generateMemorableNameFromCommit(commit *GitHubCommitInfo) string {
	// Use a simple hash-based approach for now
	adjectives := []string{"bright", "swift", "bold", "calm", "wise", "strong", "gentle", "fierce"}
	nouns := []string{"river", "mountain", "forest", "ocean", "star", "moon", "sun", "wind"}
	
	// Use first few characters of commit SHA to ensure consistency
	hash := strings.ToLower(commit.Tree.SHA)
	if len(hash) < 8 {
		hash = "00000000"
	}
	
	// Parse hex characters properly
	val1, _ := hex.DecodeString(hash[0:2])
	val2, _ := hex.DecodeString(hash[2:4])
	val3, _ := hex.DecodeString(hash[4:6])
	val4, _ := hex.DecodeString(hash[6:8])
	
	adjIndex := int(val1[0]) % len(adjectives)
	nounIndex := int(val2[0]) % len(nouns)
	number := (int(val3[0])*256 + int(val4[0])) % 1000
	
	return fmt.Sprintf("%s-%s-%d", adjectives[adjIndex], nouns[nounIndex], number)
}

// fetchFromGitLab fetches changes from a GitLab repository
func (nm *NetworkManager) fetchFromGitLab(portalURL, timeline string) (*FetchResult, error) {
	// Placeholder for GitLab implementation
	return &FetchResult{
		Refs:    []RemoteRef{},
		Seals:   []*objects.Seal{},
		Objects: []objects.Hash{},
	}, nil
}

// fetchFromIvaldiRepo fetches changes from a native Ivaldi repository
func (nm *NetworkManager) fetchFromIvaldiRepo(portalURL, timeline string) (*FetchResult, error) {
	// Placeholder for native Ivaldi protocol
	return &FetchResult{
		Refs:    []RemoteRef{},
		Seals:   []*objects.Seal{},
		Objects: []objects.Hash{},
	}, nil
}