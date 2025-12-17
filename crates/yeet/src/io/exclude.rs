use std::collections::HashSet;
use std::path::Path;

/// OS-specific default excludes for paths we never want to sync
pub struct ExcludeRules {
    /// File names to exclude
    file_excludes: HashSet<&'static str>,
    /// Directory names to exclude
    dir_excludes: HashSet<&'static str>,
}

impl ExcludeRules {
    pub fn new() -> Self {
        let file_excludes = HashSet::new();
        let mut dir_excludes = HashSet::new();

        // Unix-wide directory excludes
        #[cfg(unix)]
        {
            dir_excludes.insert("lost+found");
        }

        // macOS-specific directory excludes
        #[cfg(target_os = "macos")]
        {
            dir_excludes.insert(".fseventsd");
            dir_excludes.insert(".Trashes");
            dir_excludes.insert(".TemporaryItems");
            dir_excludes.insert(".DocumentRevisions-V100");
            dir_excludes.insert(".Spotlight-V100");
        }

        Self {
            file_excludes,
            dir_excludes,
        }
    }

    pub fn should_exclude_dir(&self, dir_name: &str) -> bool {
        self.dir_excludes.contains(dir_name)
    }

    pub fn should_exclude_file(&self, file_name: &str) -> bool {
        self.file_excludes.contains(file_name)
    }

    /// Check if a directory should be excluded by path name (for macos its
    /// unlikely we ever managed to traverse stuff like .fsenventsd but WHO
    /// KNOWS maybe we're running as root and in a weird spot)
    pub fn should_exclude_dir_path(&self, path: &Path) -> bool {
        if let Some(file_name) = path.file_name()
            && let Some(name_str) = file_name.to_str()
        {
            return self.should_exclude_dir(name_str);
        }
        false
    }

    /// Check if a file should be excluded by name.
    pub fn should_exclude_file_path(&self, path: &Path) -> bool {
        if let Some(file_name) = path.file_name()
            && let Some(name_str) = file_name.to_str()
        {
            return self.should_exclude_file(name_str);
        }
        false
    }
}

impl Default for ExcludeRules {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_unix_dir_excludes() {
        let rules = ExcludeRules::new();
        assert!(rules.should_exclude_dir("lost+found"));
        assert!(!rules.should_exclude_dir("normal_dir"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos_dir_excludes() {
        let rules = ExcludeRules::new();
        assert!(rules.should_exclude_dir(".fseventsd"));
        assert!(rules.should_exclude_dir(".Trashes"));
        assert!(!rules.should_exclude_dir(".git"));
    }

    #[test]
    #[cfg(unix)]
    fn test_dir_path_excludes() {
        let rules = ExcludeRules::new();
        let path = Path::new("/some/path/lost+found");
        assert!(rules.should_exclude_dir_path(path));

        let normal_path = Path::new("/some/path/normal");
        assert!(!rules.should_exclude_dir_path(normal_path));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos_path_excludes() {
        let rules = ExcludeRules::new();
        let path = Path::new("/Volumes/drive/.fseventsd");
        assert!(rules.should_exclude_dir_path(path));

        let trashes = Path::new("/Volumes/drive/.Trashes");
        assert!(rules.should_exclude_dir_path(trashes));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn test_macos_path_excludes_only_exact_name() {
        let rules = ExcludeRules::new();
        // Subdirectories inside excluded dirs are not checked (parent is already excluded)
        let path = Path::new("/Volumes/drive/.fseventsd/foo");
        assert!(!rules.should_exclude_dir_path(path));

        // But the excluded directory itself should be caught
        let parent = Path::new("/Volumes/drive/.fseventsd");
        assert!(rules.should_exclude_dir_path(parent));
    }
}
