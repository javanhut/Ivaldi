package index

import (
	"database/sql"
	"path/filepath"
	"sync"
	"time"

	"ivaldi/core/objects"
	_ "modernc.org/sqlite"
)

type SQLiteIndex struct {
	db   *sql.DB
	path string
	mu   sync.RWMutex // Protects database operations
}

func NewSQLiteIndex(root string) (*SQLiteIndex, error) {
	indexPath := filepath.Join(root, ".ivaldi", "index.db")

	// Configure SQLite for concurrent access
	db, err := sql.Open("sqlite", indexPath+"?_pragma=journal_mode(WAL)&_pragma=busy_timeout(30000)&_pragma=synchronous(NORMAL)&_pragma=cache_size(10000)")
	if err != nil {
		return nil, err
	}

	// Configure connection pool for better concurrency
	db.SetMaxOpenConns(10)
	db.SetMaxIdleConns(5)
	db.SetConnMaxLifetime(time.Hour)

	index := &SQLiteIndex{
		db:   db,
		path: indexPath,
	}

	if err := index.initialize(); err != nil {
		db.Close()
		return nil, err
	}

	return index, nil
}

func (idx *SQLiteIndex) initialize() error {
	schema := `
	CREATE TABLE IF NOT EXISTS objects (
		hash BLOB PRIMARY KEY,
		type INTEGER NOT NULL,
		size INTEGER NOT NULL,
		created_at INTEGER NOT NULL,
		ref_count INTEGER DEFAULT 1
	);

	CREATE TABLE IF NOT EXISTS seals (
		hash BLOB PRIMARY KEY,
		name TEXT UNIQUE NOT NULL,
		iteration INTEGER,
		position BLOB,
		message TEXT,
		author_name TEXT,
		author_email TEXT,
		timestamp INTEGER,
		parent_count INTEGER DEFAULT 0,
		FOREIGN KEY (hash) REFERENCES objects(hash)
	);

	CREATE TABLE IF NOT EXISTS seal_parents (
		seal_hash BLOB,
		parent_hash BLOB,
		position INTEGER,
		PRIMARY KEY (seal_hash, parent_hash),
		FOREIGN KEY (seal_hash) REFERENCES seals(hash),
		FOREIGN KEY (parent_hash) REFERENCES seals(hash)
	);

	CREATE TABLE IF NOT EXISTS seal_overwrites (
		seal_hash BLOB,
		previous_hash BLOB,
		reason TEXT,
		author_name TEXT,
		author_email TEXT,
		timestamp INTEGER,
		FOREIGN KEY (seal_hash) REFERENCES seals(hash)
	);

	CREATE TABLE IF NOT EXISTS chunks (
		hash BLOB PRIMARY KEY,
		size INTEGER NOT NULL,
		ref_count INTEGER DEFAULT 1,
		compressed BOOLEAN DEFAULT FALSE,
		created_at INTEGER NOT NULL,
		FOREIGN KEY (hash) REFERENCES objects(hash)
	);

	CREATE TABLE IF NOT EXISTS trees (
		hash BLOB PRIMARY KEY,
		entry_count INTEGER DEFAULT 0,
		FOREIGN KEY (hash) REFERENCES objects(hash)
	);

	CREATE TABLE IF NOT EXISTS tree_entries (
		tree_hash BLOB,
		name TEXT,
		entry_hash BLOB,
		mode INTEGER,
		type INTEGER,
		position INTEGER,
		PRIMARY KEY (tree_hash, name),
		FOREIGN KEY (tree_hash) REFERENCES trees(hash)
	);

	CREATE TABLE IF NOT EXISTS timelines (
		name TEXT PRIMARY KEY,
		head_hash BLOB,
		created_at INTEGER,
		updated_at INTEGER,
		description TEXT,
		parent TEXT
	);

	CREATE INDEX IF NOT EXISTS idx_seals_name ON seals(name);
	CREATE INDEX IF NOT EXISTS idx_seals_iteration ON seals(iteration);
	CREATE INDEX IF NOT EXISTS idx_seals_timestamp ON seals(timestamp);
	CREATE INDEX IF NOT EXISTS idx_objects_type ON objects(type);
	CREATE INDEX IF NOT EXISTS idx_chunks_ref_count ON chunks(ref_count);
	CREATE INDEX IF NOT EXISTS idx_timelines_head ON timelines(head_hash);
	`

	_, err := idx.db.Exec(schema)
	return err
}

func (idx *SQLiteIndex) IndexSeal(seal *objects.Seal) error {
	idx.mu.Lock()
	defer idx.mu.Unlock()

	tx, err := idx.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()

	if err := idx.indexObject(tx, seal.Hash, objects.ObjectTypeSeal, 0); err != nil {
		return err
	}

	_, err = tx.Exec(`
		INSERT OR REPLACE INTO seals 
		(hash, name, iteration, position, message, author_name, author_email, timestamp, parent_count)
		VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`,
		seal.Hash[:], seal.Name, seal.Iteration, seal.Position[:],
		seal.Message, seal.Author.Name, seal.Author.Email,
		seal.Timestamp.Unix(), len(seal.Parents))
	if err != nil {
		return err
	}

	for i, parent := range seal.Parents {
		_, err = tx.Exec(`
			INSERT OR REPLACE INTO seal_parents (seal_hash, parent_hash, position)
			VALUES (?, ?, ?)`,
			seal.Hash[:], parent[:], i)
		if err != nil {
			return err
		}
	}

	for _, overwrite := range seal.Overwrites {
		_, err = tx.Exec(`
			INSERT INTO seal_overwrites 
			(seal_hash, previous_hash, reason, author_name, author_email, timestamp)
			VALUES (?, ?, ?, ?, ?, ?)`,
			seal.Hash[:], overwrite.PreviousHash[:], overwrite.Reason,
			overwrite.Author.Name, overwrite.Author.Email, overwrite.Timestamp.Unix())
		if err != nil {
			return err
		}
	}

	return tx.Commit()
}

func (idx *SQLiteIndex) IndexChunk(chunk *objects.Chunk) error {
	idx.mu.Lock()
	defer idx.mu.Unlock()

	tx, err := idx.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()

	if err := idx.indexObject(tx, chunk.ID, objects.ObjectTypeChunk, chunk.Size); err != nil {
		return err
	}

	_, err = tx.Exec(`
		INSERT OR REPLACE INTO chunks (hash, size, ref_count, compressed, created_at)
		VALUES (?, ?, ?, ?, ?)`,
		chunk.ID[:], chunk.Size, chunk.RefCount, chunk.Compressed, time.Now().Unix())
	if err != nil {
		return err
	}

	return tx.Commit()
}

func (idx *SQLiteIndex) IndexTree(tree *objects.Tree) error {
	idx.mu.Lock()
	defer idx.mu.Unlock()

	tx, err := idx.db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()

	if err := idx.indexObject(tx, tree.Hash, objects.ObjectTypeTree, 0); err != nil {
		return err
	}

	_, err = tx.Exec(`
		INSERT OR REPLACE INTO trees (hash, entry_count)
		VALUES (?, ?)`,
		tree.Hash[:], len(tree.Entries))
	if err != nil {
		return err
	}

	for i, entry := range tree.Entries {
		_, err = tx.Exec(`
			INSERT OR REPLACE INTO tree_entries 
			(tree_hash, name, entry_hash, mode, type, position)
			VALUES (?, ?, ?, ?, ?, ?)`,
			tree.Hash[:], entry.Name, entry.Hash[:],
			entry.Mode, int(entry.Type), i)
		if err != nil {
			return err
		}
	}

	return tx.Commit()
}

func (idx *SQLiteIndex) indexObject(tx *sql.Tx, hash objects.Hash, objType objects.ObjectType, size int64) error {
	_, err := tx.Exec(`
		INSERT OR IGNORE INTO objects (hash, type, size, created_at)
		VALUES (?, ?, ?, ?)`,
		hash[:], int(objType), size, time.Now().Unix())
	return err
}

func (idx *SQLiteIndex) FindSealByName(name string) (*objects.Hash, error) {
	var hash []byte
	err := idx.db.QueryRow(`
		SELECT hash FROM seals WHERE name = ?`, name).Scan(&hash)
	if err != nil {
		return nil, err
	}

	var result objects.Hash
	copy(result[:], hash)
	return &result, nil
}

func (idx *SQLiteIndex) FindSealByIteration(iteration int) (*objects.Hash, error) {
	var hash []byte
	err := idx.db.QueryRow(`
		SELECT hash FROM seals WHERE iteration = ?`, iteration).Scan(&hash)
	if err != nil {
		return nil, err
	}

	var result objects.Hash
	copy(result[:], hash)
	return &result, nil
}

func (idx *SQLiteIndex) GetSealHistory(limit int) ([]objects.Hash, error) {
	rows, err := idx.db.Query(`
		SELECT hash FROM seals 
		ORDER BY timestamp DESC 
		LIMIT ?`, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var hashes []objects.Hash
	for rows.Next() {
		var hash []byte
		if err := rows.Scan(&hash); err != nil {
			return nil, err
		}

		var h objects.Hash
		copy(h[:], hash)
		hashes = append(hashes, h)
	}

	return hashes, rows.Err()
}

func (idx *SQLiteIndex) GetChunkStats() (ChunkStats, error) {
	var stats ChunkStats

	err := idx.db.QueryRow(`
		SELECT 
			COUNT(*) as count,
			SUM(size) as total_size,
			SUM(CASE WHEN ref_count > 1 THEN size ELSE 0 END) as deduplicated_size
		FROM chunks`).Scan(&stats.Count, &stats.TotalSize, &stats.DeduplicatedSize)

	if err != nil {
		return stats, err
	}

	if stats.TotalSize > 0 {
		stats.DeduplicationRatio = float64(stats.TotalSize) / float64(stats.TotalSize-stats.DeduplicatedSize)
	}

	return stats, nil
}

func (idx *SQLiteIndex) IncrementChunkRef(hash objects.Hash) error {
	_, err := idx.db.Exec(`
		UPDATE chunks SET ref_count = ref_count + 1 WHERE hash = ?`, hash[:])
	return err
}

func (idx *SQLiteIndex) DecrementChunkRef(hash objects.Hash) error {
	_, err := idx.db.Exec(`
		UPDATE chunks SET ref_count = ref_count - 1 WHERE hash = ?`, hash[:])
	return err
}

func (idx *SQLiteIndex) GetUnreferencedChunks() ([]objects.Hash, error) {
	rows, err := idx.db.Query(`
		SELECT hash FROM chunks WHERE ref_count <= 0`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var hashes []objects.Hash
	for rows.Next() {
		var hash []byte
		if err := rows.Scan(&hash); err != nil {
			return nil, err
		}

		var h objects.Hash
		copy(h[:], hash)
		hashes = append(hashes, h)
	}

	return hashes, rows.Err()
}

func (idx *SQLiteIndex) Vacuum() error {
	_, err := idx.db.Exec("VACUUM")
	return err
}

func (idx *SQLiteIndex) Close() error {
	return idx.db.Close()
}

// Natural language reference resolution methods
func (idx *SQLiteIndex) FindSealsByAuthor(author string) ([]objects.Hash, error) {
	rows, err := idx.db.Query(`
		SELECT hash FROM seals 
		WHERE author_name = ? 
		ORDER BY timestamp DESC`, author)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var hashes []objects.Hash
	for rows.Next() {
		var hash []byte
		if err := rows.Scan(&hash); err != nil {
			return nil, err
		}

		var h objects.Hash
		copy(h[:], hash)
		hashes = append(hashes, h)
	}

	return hashes, rows.Err()
}

func (idx *SQLiteIndex) FindSealsByTimeRange(start, end time.Time) ([]objects.Hash, error) {
	rows, err := idx.db.Query(`
		SELECT hash FROM seals 
		WHERE timestamp BETWEEN ? AND ?
		ORDER BY timestamp DESC`, start.Unix(), end.Unix())
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var hashes []objects.Hash
	for rows.Next() {
		var hash []byte
		if err := rows.Scan(&hash); err != nil {
			return nil, err
		}

		var h objects.Hash
		copy(h[:], hash)
		hashes = append(hashes, h)
	}

	return hashes, rows.Err()
}

func (idx *SQLiteIndex) FindSealsContaining(searchTerm string) ([]objects.Hash, error) {
	rows, err := idx.db.Query(`
		SELECT hash FROM seals 
		WHERE message LIKE ?
		ORDER BY timestamp DESC`, "%"+searchTerm+"%")
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var hashes []objects.Hash
	for rows.Next() {
		var hash []byte
		if err := rows.Scan(&hash); err != nil {
			return nil, err
		}

		var h objects.Hash
		copy(h[:], hash)
		hashes = append(hashes, h)
	}

	return hashes, rows.Err()
}

func (idx *SQLiteIndex) FindSealByHashPrefix(prefix string) (*objects.Hash, error) {
	var hash []byte
	err := idx.db.QueryRow(`
		SELECT hash FROM seals 
		WHERE hex(hash) LIKE ? 
		ORDER BY timestamp DESC 
		LIMIT 1`, prefix+"%").Scan(&hash)
	if err != nil {
		return nil, err
	}

	var result objects.Hash
	copy(result[:], hash)
	return &result, nil
}

func (idx *SQLiteIndex) GetSealByTimeline(timeline string, iteration int) (*objects.Hash, error) {
	// For now, ignore timeline and just use global iteration
	// TODO: Implement timeline-specific iteration tracking
	return idx.FindSealByIteration(iteration)
}

type ChunkStats struct {
	Count              int64
	TotalSize          int64
	DeduplicatedSize   int64
	DeduplicationRatio float64
}
