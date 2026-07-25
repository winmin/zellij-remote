use glob::glob;
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

pub const MAX_INCLUDE_DEPTH: usize = 16;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParseResult {
    pub hosts: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn parse_ssh_config(config_path: &Path, home: &Path) -> ParseResult {
    let mut parser = Parser {
        home,
        include_base: config_path.parent().unwrap_or_else(|| Path::new(".")),
        result: ParseResult::default(),
        seen_hosts: HashSet::new(),
        visited: HashSet::new(),
    };
    parser.parse_file(config_path, 0);
    parser.result
}

struct Parser<'a> {
    home: &'a Path,
    include_base: &'a Path,
    result: ParseResult,
    seen_hosts: HashSet<String>,
    visited: HashSet<PathBuf>,
}

impl Parser<'_> {
    fn parse_file(&mut self, path: &Path, depth: usize) {
        if depth > MAX_INCLUDE_DEPTH {
            self.result.warnings.push(format!(
                "maximum Include depth ({MAX_INCLUDE_DEPTH}) exceeded at {}",
                path.display()
            ));
            return;
        }

        let visit_key = fs::canonicalize(path).unwrap_or_else(|_| normalize_path(path));
        if !self.visited.insert(visit_key) {
            return;
        }

        let contents = match fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) => {
                self.result
                    .warnings
                    .push(format!("{}: {error}", path.display()));
                return;
            }
        };

        for (line_index, line) in contents.lines().enumerate() {
            let tokens = match tokenize(line) {
                Ok(tokens) => tokens,
                Err(error) => {
                    self.result.warnings.push(format!(
                        "{}:{}: {error}",
                        path.display(),
                        line_index + 1
                    ));
                    continue;
                }
            };
            if tokens.is_empty() {
                continue;
            }

            let (keyword, attached_value) = match tokens[0].split_once('=') {
                Some((keyword, value)) => (keyword, Some(value)),
                None => (tokens[0].as_str(), None),
            };
            let arguments: Vec<&str> = attached_value
                .filter(|value| !value.is_empty())
                .into_iter()
                .chain(tokens[1..].iter().map(String::as_str))
                .collect();

            if keyword.eq_ignore_ascii_case("host") {
                for alias in arguments {
                    if is_concrete_alias(alias) && self.seen_hosts.insert(alias.to_owned()) {
                        self.result.hosts.push(alias.to_owned());
                    }
                }
            } else if keyword.eq_ignore_ascii_case("include") {
                for pattern in arguments {
                    self.parse_include(pattern, depth + 1);
                }
            }
        }
    }

    fn parse_include(&mut self, pattern: &str, depth: usize) {
        let expanded = if pattern == "~" {
            self.home.to_path_buf()
        } else if let Some(rest) = pattern.strip_prefix("~/") {
            self.home.join(rest)
        } else {
            let path = Path::new(pattern);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                self.include_base.join(path)
            }
        };

        let pattern = expanded.to_string_lossy();
        let entries = match glob(&pattern) {
            Ok(entries) => entries,
            Err(error) => {
                self.result.warnings.push(format!(
                    "invalid Include pattern {}: {error}",
                    expanded.display()
                ));
                return;
            }
        };

        let mut matches = Vec::new();
        for entry in entries {
            match entry {
                Ok(path) => matches.push(path),
                Err(error) => self
                    .result
                    .warnings
                    .push(format!("Include glob error: {error}")),
            }
        }
        matches.sort();
        for path in matches {
            self.parse_file(&path, depth);
        }
    }
}

fn is_concrete_alias(alias: &str) -> bool {
    !alias.is_empty()
        && !alias
            .chars()
            .any(|character| matches!(character, '*' | '?' | '!'))
}

fn tokenize(line: &str) -> Result<Vec<String>, &'static str> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for character in line.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' {
            escaped = true;
            continue;
        }
        if let Some(delimiter) = quote {
            if character == delimiter {
                quote = None;
            } else {
                current.push(character);
            }
            continue;
        }
        match character {
            '\'' | '"' => quote = Some(character),
            '#' => break,
            character if character.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
    }

    if escaped {
        current.push('\\');
    }
    if quote.is_some() {
        return Err("unterminated quote");
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let id = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "zellij-ssh-manager-parser-{}-{id}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative_path: &str, contents: &str) -> PathBuf {
            let path = self.0.join(relative_path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_multiple_aliases_quotes_comments_and_patterns() {
        let dir = TestDir::new();
        let config = dir.write(
            ".ssh/config",
            "Host alpha \"quoted-host\" *.example !blocked question? # ignored\n\
             Host alpha beta\\ host\n\
             Host='equals-host'\n\
             Host 'unterminated\n\
             Host omega\n",
        );

        let parsed = parse_ssh_config(&config, &dir.0);

        assert_eq!(
            parsed.hosts,
            vec!["alpha", "quoted-host", "beta host", "equals-host", "omega"]
        );
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].contains("unterminated quote"));
    }

    #[test]
    fn expands_recursive_include_globs_in_sorted_order() {
        let dir = TestDir::new();
        let config = dir.write(
            ".ssh/config",
            "Host first\nInclude conf.d/*.conf\nHost last\n",
        );
        dir.write(".ssh/conf.d/20.conf", "Host twenty\n");
        dir.write(".ssh/conf.d/10.conf", "Host ten\nInclude nested/*.conf\n");
        dir.write(".ssh/nested/deep.conf", "Host deep\n");

        let parsed = parse_ssh_config(&config, &dir.0);

        assert_eq!(parsed.hosts, vec!["first", "ten", "deep", "twenty", "last"]);
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn expands_home_include_and_deduplicates_in_first_seen_order() {
        let dir = TestDir::new();
        let config = dir.write(
            ".ssh/config",
            "Host shared root\nInclude ~/.ssh/extra\nHost tail shared\n",
        );
        dir.write(".ssh/extra", "Host included shared\n");

        let parsed = parse_ssh_config(&config, &dir.0);

        assert_eq!(parsed.hosts, vec!["shared", "root", "included", "tail"]);
    }

    #[test]
    fn include_cycles_are_ignored() {
        let dir = TestDir::new();
        let config = dir.write(".ssh/config", "Host root\nInclude a.conf\n");
        dir.write(".ssh/a.conf", "Host a\nInclude b.conf\n");
        dir.write(".ssh/b.conf", "Host b\nInclude a.conf\n");

        let parsed = parse_ssh_config(&config, &dir.0);

        assert_eq!(parsed.hosts, vec!["root", "a", "b"]);
        assert!(parsed.warnings.is_empty());
    }

    #[test]
    fn reports_missing_root_without_panicking() {
        let dir = TestDir::new();
        let parsed = parse_ssh_config(&dir.0.join("missing"), &dir.0);

        assert!(parsed.hosts.is_empty());
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].contains("missing"));
    }

    #[test]
    fn limits_include_depth() {
        let dir = TestDir::new();
        for depth in 0..=MAX_INCLUDE_DEPTH + 1 {
            let next = if depth <= MAX_INCLUDE_DEPTH {
                format!("Include {}.conf\n", depth + 1)
            } else {
                String::new()
            };
            dir.write(
                &format!(".ssh/{depth}.conf"),
                &format!("Host host-{depth}\n{next}"),
            );
        }

        let parsed = parse_ssh_config(&dir.0.join(".ssh/0.conf"), &dir.0);

        assert_eq!(parsed.hosts.len(), MAX_INCLUDE_DEPTH + 1);
        assert!(parsed
            .warnings
            .iter()
            .any(|warning| warning.contains("maximum Include depth")));
    }
}
