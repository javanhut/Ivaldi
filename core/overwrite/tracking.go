package overwrite

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"ivaldi/core/objects"
)

// OverwriteRecord tracks every history modification
type OverwriteRecord struct {
	ID                string       `json:"id"`
	OriginalHash      objects.Hash `json:"originalHash"`
	OriginalName      string       `json:"originalName"`
	NewHash           objects.Hash `json:"newHash"`
	NewName           string       `json:"newName"`
	Justification     string       `json:"justification"`
	Category          string       `json:"category"` // security, cleanup, mistake, refactor
	Author            string       `json:"author"`
	Timestamp         time.Time    `json:"timestamp"`
	AffectedCommits   []string     `json:"affectedCommits"`
	ArchivedVersions  []string     `json:"archivedVersions"`
	NotifiedAuthors   []string     `json:"notifiedAuthors"`
	RequiredApproval  bool         `json:"requiredApproval"`
	ApprovalStatus    string       `json:"approvalStatus"`
}

// OverwriteCategory defines valid justification categories
type OverwriteCategory string

const (
	CategorySecurity  OverwriteCategory = "security"
	CategoryCleanup   OverwriteCategory = "cleanup"
	CategoryMistake   OverwriteCategory = "mistake"
	CategoryRefactor  OverwriteCategory = "refactor"
	CategoryRebase    OverwriteCategory = "rebase"
	CategorySquash    OverwriteCategory = "squash"
	CategoryAmend     OverwriteCategory = "amend"
)

// OverwriteTracker manages history modification tracking
type OverwriteTracker struct {
	root    string
	records map[string]*OverwriteRecord
	config  *OverwriteConfig
}

type OverwriteConfig struct {
	RequireJustification   bool   `json:"requireJustification"`
	MinJustificationLength int    `json:"minJustificationLength"`
	NotifyAuthors          bool   `json:"notifyAuthors"`
	ProtectReleases        bool   `json:"protectReleases"`
	AuditRetentionDays     int    `json:"auditRetentionDays"`
	RequireApproval        bool   `json:"requireApproval"`
	ProtectedCommits       []string `json:"protectedCommits"`
}

func NewOverwriteTracker(root string) *OverwriteTracker {
	return &OverwriteTracker{
		root:    root,
		records: make(map[string]*OverwriteRecord),
		config: &OverwriteConfig{
			RequireJustification:   true,
			MinJustificationLength: 20,
			NotifyAuthors:          true,
			ProtectReleases:        true,
			AuditRetentionDays:     365,
			RequireApproval:        false,
			ProtectedCommits:       []string{},
		},
	}
}

// RequestOverwrite initiates the overwrite process with justification
func (ot *OverwriteTracker) RequestOverwrite(
	originalHash objects.Hash,
	originalName string,
	newHash objects.Hash,
	newName string,
	justification string,
	category OverwriteCategory,
	author string,
) (*OverwriteRecord, error) {
	
	// Validate justification
	if ot.config.RequireJustification {
		if len(justification) < ot.config.MinJustificationLength {
			return nil, fmt.Errorf("justification must be at least %d characters", ot.config.MinJustificationLength)
		}
	}
	
	// Check if commit is protected
	if ot.isProtected(originalHash) {
		return nil, fmt.Errorf("commit %s is protected and cannot be overwritten", originalName)
	}
	
	// Create overwrite record
	record := &OverwriteRecord{
		ID:               ot.generateRecordID(),
		OriginalHash:     originalHash,
		OriginalName:     originalName,
		NewHash:          newHash,
		NewName:          newName,
		Justification:    justification,
		Category:         string(category),
		Author:           author,
		Timestamp:        time.Now(),
		AffectedCommits:  []string{originalName},
		ArchivedVersions: []string{},
		NotifiedAuthors:  []string{},
		RequiredApproval: ot.config.RequireApproval,
		ApprovalStatus:   "pending",
	}
	
	// Archive original version
	if err := ot.archiveVersion(record); err != nil {
		return nil, fmt.Errorf("failed to archive original version: %v", err)
	}
	
	// Store record
	ot.records[record.ID] = record
	
	if err := ot.saveRecord(record); err != nil {
		return nil, fmt.Errorf("failed to save overwrite record: %v", err)
	}
	
	// Notify affected authors
	if ot.config.NotifyAuthors {
		if err := ot.notifyAuthors(record); err != nil {
			// Log error but don't fail the operation
			fmt.Printf("Warning: failed to notify authors: %v\n", err)
		}
	}
	
	return record, nil
}

// GetOverwriteHistory returns overwrite records for a commit
func (ot *OverwriteTracker) GetOverwriteHistory(commitName string) []*OverwriteRecord {
	var history []*OverwriteRecord
	
	for _, record := range ot.records {
		for _, affected := range record.AffectedCommits {
			if affected == commitName {
				history = append(history, record)
				break
			}
		}
	}
	
	return history
}

// GetOverwriteCount returns the number of times a commit has been overwritten
func (ot *OverwriteTracker) GetOverwriteCount(commitName string) int {
	return len(ot.GetOverwriteHistory(commitName))
}

// GetArchivedVersions returns all archived versions of a commit
func (ot *OverwriteTracker) GetArchivedVersions(commitName string) []string {
	var versions []string
	
	for _, record := range ot.records {
		for _, affected := range record.AffectedCommits {
			if affected == commitName {
				versions = append(versions, record.ArchivedVersions...)
			}
		}
	}
	
	return versions
}

// ApproveOverwrite approves a pending overwrite operation
func (ot *OverwriteTracker) ApproveOverwrite(recordID string, approver string) error {
	record, exists := ot.records[recordID]
	if !exists {
		return fmt.Errorf("overwrite record %s not found", recordID)
	}
	
	if record.ApprovalStatus != "pending" {
		return fmt.Errorf("overwrite %s is not pending approval", recordID)
	}
	
	record.ApprovalStatus = "approved"
	record.NotifiedAuthors = append(record.NotifiedAuthors, approver)
	
	return ot.saveRecord(record)
}

// RejectOverwrite rejects a pending overwrite operation
func (ot *OverwriteTracker) RejectOverwrite(recordID string, rejector string, reason string) error {
	record, exists := ot.records[recordID]
	if !exists {
		return fmt.Errorf("overwrite record %s not found", recordID)
	}
	
	if record.ApprovalStatus != "pending" {
		return fmt.Errorf("overwrite %s is not pending approval", recordID)
	}
	
	record.ApprovalStatus = "rejected"
	record.Justification += fmt.Sprintf("\n[REJECTED by %s: %s]", rejector, reason)
	
	return ot.saveRecord(record)
}

// ProtectCommit marks a commit as protected
func (ot *OverwriteTracker) ProtectCommit(commitHash objects.Hash) error {
	hashStr := fmt.Sprintf("%x", commitHash)
	
	for _, protected := range ot.config.ProtectedCommits {
		if protected == hashStr {
			return nil // Already protected
		}
	}
	
	ot.config.ProtectedCommits = append(ot.config.ProtectedCommits, hashStr)
	return ot.saveConfig()
}

// UnprotectCommit removes protection from a commit
func (ot *OverwriteTracker) UnprotectCommit(commitHash objects.Hash) error {
	hashStr := fmt.Sprintf("%x", commitHash)
	
	for i, protected := range ot.config.ProtectedCommits {
		if protected == hashStr {
			ot.config.ProtectedCommits = append(
				ot.config.ProtectedCommits[:i],
				ot.config.ProtectedCommits[i+1:]...,
			)
			return ot.saveConfig()
		}
	}
	
	return nil // Wasn't protected anyway
}

// ExportAuditTrail exports the complete audit trail for compliance
func (ot *OverwriteTracker) ExportAuditTrail() ([]byte, error) {
	auditData := struct {
		Config  *OverwriteConfig            `json:"config"`
		Records map[string]*OverwriteRecord `json:"records"`
		Export  struct {
			Timestamp time.Time `json:"timestamp"`
			Version   string    `json:"version"`
		} `json:"export"`
	}{
		Config:  ot.config,
		Records: ot.records,
	}
	
	auditData.Export.Timestamp = time.Now()
	auditData.Export.Version = "1.0"
	
	return json.MarshalIndent(auditData, "", "  ")
}

// Helper functions

func (ot *OverwriteTracker) isProtected(commitHash objects.Hash) bool {
	hashStr := fmt.Sprintf("%x", commitHash)
	
	for _, protected := range ot.config.ProtectedCommits {
		if protected == hashStr {
			return true
		}
	}
	
	return false
}

func (ot *OverwriteTracker) archiveVersion(record *OverwriteRecord) error {
	// Create archive directory
	archiveDir := filepath.Join(ot.root, ".ivaldi", "archive")
	if err := os.MkdirAll(archiveDir, 0755); err != nil {
		return err
	}
	
	// Archive the original commit
	archiveName := fmt.Sprintf("%s.v%d", record.OriginalName, len(record.ArchivedVersions)+1)
	archivePath := filepath.Join(archiveDir, archiveName+".json")
	
	archiveData := struct {
		RecordID     string       `json:"recordId"`
		OriginalHash objects.Hash `json:"originalHash"`
		OriginalName string       `json:"originalName"`
		ArchivedAt   time.Time    `json:"archivedAt"`
		Reason       string       `json:"reason"`
	}{
		RecordID:     record.ID,
		OriginalHash: record.OriginalHash,
		OriginalName: record.OriginalName,
		ArchivedAt:   time.Now(),
		Reason:       record.Justification,
	}
	
	data, err := json.MarshalIndent(archiveData, "", "  ")
	if err != nil {
		return err
	}
	
	if err := os.WriteFile(archivePath, data, 0644); err != nil {
		return err
	}
	
	record.ArchivedVersions = append(record.ArchivedVersions, archiveName)
	return nil
}

func (ot *OverwriteTracker) notifyAuthors(record *OverwriteRecord) error {
	// TODO: Implement author notification system
	// This would typically send emails or create notifications
	fmt.Printf("NOTIFICATION: Commit %s has been overwritten\n", record.OriginalName)
	fmt.Printf("Reason: %s\n", record.Justification)
	fmt.Printf("New version: %s\n", record.NewName)
	fmt.Printf("Archive available: %s\n", record.ArchivedVersions)
	
	return nil
}

func (ot *OverwriteTracker) generateRecordID() string {
	return fmt.Sprintf("ow_%d", time.Now().UnixNano())
}

func (ot *OverwriteTracker) saveRecord(record *OverwriteRecord) error {
	recordDir := filepath.Join(ot.root, ".ivaldi", "overwrites")
	if err := os.MkdirAll(recordDir, 0755); err != nil {
		return err
	}
	
	recordPath := filepath.Join(recordDir, record.ID+".json")
	
	data, err := json.MarshalIndent(record, "", "  ")
	if err != nil {
		return err
	}
	
	return os.WriteFile(recordPath, data, 0644)
}

func (ot *OverwriteTracker) saveConfig() error {
	configPath := filepath.Join(ot.root, ".ivaldi", "overwrite-config.json")
	
	data, err := json.MarshalIndent(ot.config, "", "  ")
	if err != nil {
		return err
	}
	
	return os.WriteFile(configPath, data, 0644)
}

// Load loads all overwrite records from disk
func (ot *OverwriteTracker) Load() error {
	// Load config
	configPath := filepath.Join(ot.root, ".ivaldi", "overwrite-config.json")
	if data, err := os.ReadFile(configPath); err == nil {
		json.Unmarshal(data, ot.config)
	}
	
	// Load records
	recordDir := filepath.Join(ot.root, ".ivaldi", "overwrites")
	if _, err := os.Stat(recordDir); os.IsNotExist(err) {
		return nil
	}
	
	files, err := os.ReadDir(recordDir)
	if err != nil {
		return err
	}
	
	for _, file := range files {
		if !strings.HasSuffix(file.Name(), ".json") {
			continue
		}
		
		recordPath := filepath.Join(recordDir, file.Name())
		data, err := os.ReadFile(recordPath)
		if err != nil {
			continue
		}
		
		var record OverwriteRecord
		if err := json.Unmarshal(data, &record); err != nil {
			continue
		}
		
		ot.records[record.ID] = &record
	}
	
	return nil
}