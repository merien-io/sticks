CREATE TABLE IF NOT EXISTS github_cache (
    id INTEGER PRIMARY KEY,
    repo_json TEXT,
    commit_json TEXT,
    updated_at INTEGER
);
