package context

import (
	"os"
	"path/filepath"
	"strings"
)

// Loader discovers and loads project context files.
type Loader struct {
	cwd string
}

// NewLoader creates a loader rooted at the given directory.
func NewLoader(cwd string) *Loader {
	return &Loader{cwd: cwd}
}

// Load walks from cwd up to the git worktree root (or home), collecting
// AGENTS.md, CLAUDE.md, and .mew/AGENTS.md along the way. It also loads
// ~/.config/mew/AGENTS.md if present. Files are returned from most-general
// to most-specific.
func (l *Loader) Load() ([]File, error) {
	var files []File

	// Global config file first.
	if cfgDir, err := os.UserConfigDir(); err == nil {
		p := filepath.Join(cfgDir, "mew", "AGENTS.md")
		if data, err := os.ReadFile(p); err == nil {
			files = append(files, File{Path: p, Content: string(data)})
		}
	}

	// Walk up from cwd to git root or home.
	root, err := l.findGitRoot(l.cwd)
	if err != nil {
		root = l.cwd
	}

	// Collect paths from root down to cwd so that most-general comes first.
	paths := l.pathsBetween(root, l.cwd)
	for _, dir := range paths {
		for _, name := range []string{"AGENTS.md", "CLAUDE.md"} {
			p := filepath.Join(dir, name)
			if data, err := os.ReadFile(p); err == nil {
				files = append(files, File{Path: p, Content: string(data)})
			}
		}
		p := filepath.Join(dir, ".mew", "AGENTS.md")
		if data, err := os.ReadFile(p); err == nil {
			files = append(files, File{Path: p, Content: string(data)})
		}
	}

	return files, nil
}

// File is a loaded context file.
type File struct {
	Path    string
	Content string
}

// BuildSystemPrompt concatenates the loaded files into a system prompt fragment.
func BuildSystemPrompt(files []File) string {
	var b strings.Builder
	for _, f := range files {
		b.WriteString("<context source=\"")
		b.WriteString(f.Path)
		b.WriteString("\">\n")
		b.WriteString(f.Content)
		b.WriteString("\n</context>\n")
	}
	return b.String()
}

func (l *Loader) findGitRoot(dir string) (string, error) {
	for {
		if _, err := os.Stat(filepath.Join(dir, ".git")); err == nil {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			return "", os.ErrNotExist
		}
		dir = parent
	}
}

func (l *Loader) pathsBetween(root, leaf string) []string {
	// If leaf is inside root, collect all directories from root to leaf.
	// Otherwise just return leaf.
	var out []string
	if !strings.HasPrefix(leaf, root) {
		return []string{leaf}
	}
	cur := root
	for {
		out = append(out, cur)
		if cur == leaf {
			break
		}
		// Find the next directory component in leaf after cur.
		suffix := strings.TrimPrefix(leaf, cur)
		suffix = strings.TrimPrefix(suffix, string(filepath.Separator))
		if suffix == "" {
			break
		}
		parts := strings.SplitN(suffix, string(filepath.Separator), 2)
		cur = filepath.Join(cur, parts[0])
	}
	return out
}
