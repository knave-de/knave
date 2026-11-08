//! Freedesktop `.desktop` parsing and `Exec` expansion.

/// Fields of the `[Desktop Entry]` group that the launcher consumes.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Entry {
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub keywords: Vec<String>,
    pub exec: String,
    pub icon: Option<String>,
    pub try_exec: Option<String>,
    pub is_application: bool,
    pub no_display: bool,
    /// `Hidden=true` marks the entry as deleted, which also masks lower-priority copies.
    pub deleted: bool,
    pub terminal: bool,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
}

/// A localized value keeps the best-ranked locale seen so far.
struct Localized {
    rank: usize,
    value: String,
}
fn offer(slot: &mut Option<Localized>, rank: usize, value: String) {
    if slot.as_ref().is_none_or(|current| rank < current.rank) {
        *slot = Some(Localized { rank, value });
    }
}

pub(crate) fn parse(text: &str, locales: &[String]) -> Entry {
    let mut entry = Entry::default();
    let (mut name, mut generic, mut comment, mut keywords) = (None, None, None, None);
    let mut in_entry = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            // Later groups are actions; only the main group describes the application.
            if in_entry {
                break;
            }
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, locale) = match key.trim().split_once('[') {
            Some((key, tag)) => (key, tag.strip_suffix(']')),
            None => (key.trim(), None),
        };
        // Unlocalized values rank below every requested locale.
        let rank = match locale {
            None => locales.len(),
            Some(tag) => match locales.iter().position(|l| l == tag) {
                Some(rank) => rank,
                None => continue,
            },
        };
        let value = value.trim();
        match key {
            "Name" => offer(&mut name, rank, unescape(value)),
            "GenericName" => offer(&mut generic, rank, unescape(value)),
            "Comment" => offer(&mut comment, rank, unescape(value)),
            "Keywords" => offer(&mut keywords, rank, value.to_owned()),
            _ if locale.is_some() => {}
            "Type" => entry.is_application = value == "Application",
            "Exec" => entry.exec = unescape(value),
            "Icon" => entry.icon = Some(unescape(value)).filter(|v| !v.is_empty()),
            "TryExec" => entry.try_exec = Some(unescape(value)),
            "NoDisplay" => entry.no_display = value == "true",
            "Hidden" => entry.deleted = value == "true",
            "Terminal" => entry.terminal = value == "true",
            "OnlyShowIn" => entry.only_show_in = split_list(value),
            "NotShowIn" => entry.not_show_in = split_list(value),
            _ => {}
        }
    }
    entry.name = name.map(|v| v.value).unwrap_or_default();
    entry.generic_name = generic.map(|v| v.value).unwrap_or_default();
    entry.comment = comment.map(|v| v.value).unwrap_or_default();
    entry.keywords = keywords.map(|v| split_list(&v.value)).unwrap_or_default();
    entry
}

/// Locale tags to try, best first: `ll_CC@MOD`, `ll_CC`, `ll@MOD`, `ll`.
pub(crate) fn locale_candidates(raw: &str) -> Vec<String> {
    // POSIX form: lang[_COUNTRY][.ENCODING][@MODIFIER]; the encoding never matches a tag.
    let (head, modifier) = raw
        .split_once('@')
        .map_or((raw, None), |(h, m)| (h, Some(m)));
    let base = head.split('.').next().unwrap_or_default();
    let (lang, country) = base
        .split_once('_')
        .map_or((base, None), |(l, c)| (l, Some(c)));
    if lang.is_empty() || lang == "C" || lang == "POSIX" {
        return Vec::new();
    }
    let mut tags = Vec::new();
    if let (Some(c), Some(m)) = (country, modifier) {
        tags.push(format!("{lang}_{c}@{m}"));
    }
    if let Some(c) = country {
        tags.push(format!("{lang}_{c}"));
    }
    if let Some(m) = modifier {
        tags.push(format!("{lang}@{m}"));
    }
    tags.push(lang.to_owned());
    tags
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Split a `;` separated list; `\;` is a literal semicolon.
fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => current.push(';'),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            ';' => items.push(std::mem::take(&mut current)),
            c => current.push(c),
        }
    }
    items.push(current);
    items
        .into_iter()
        .map(|item| unescape(item.trim()))
        .filter(|item| !item.is_empty())
        .collect()
}

/// Expand `Exec` into an argument vector; file/URL field codes expand to nothing.
pub(crate) fn argv(exec: &str, name: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut chars = exec.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        if chars.peek().is_none() {
            break;
        }
        let mut token = String::new();
        let mut in_quotes = false;
        let mut only_codes = true;
        while let Some(&c) = chars.peek() {
            if !in_quotes && c.is_whitespace() {
                break;
            }
            chars.next();
            match c {
                '"' => in_quotes = !in_quotes,
                '\\' if in_quotes => {
                    only_codes = false;
                    match chars.next() {
                        Some(escaped @ ('"' | '`' | '$' | '\\')) => token.push(escaped),
                        Some(other) => {
                            token.push('\\');
                            token.push(other);
                        }
                        None => token.push('\\'),
                    }
                }
                '%' if !in_quotes => match chars.next() {
                    Some('%') => {
                        only_codes = false;
                        token.push('%');
                    }
                    Some('c') => {
                        only_codes = false;
                        token.push_str(name);
                    }
                    Some(_) | None => {}
                },
                c => {
                    only_codes = false;
                    token.push(c);
                }
            }
        }
        if in_quotes {
            return None;
        }
        if !(only_codes && token.is_empty()) {
            args.push(token);
        }
    }
    (!args.is_empty() && !args[0].is_empty()).then_some(args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn locales() -> Vec<String> {
        locale_candidates("de_AT.UTF-8")
    }

    #[test]
    fn parses_main_group_and_ignores_actions() {
        let entry = parse(
            "# c\n[Desktop Entry]\nType=Application\nName=Files\nExec=files %U\nKeywords=a;b\\;c;\n[Desktop Action x]\nName=Other\nExec=other\n",
            &[],
        );
        assert!(entry.is_application);
        assert_eq!(entry.name, "Files");
        assert_eq!(entry.exec, "files %U");
        assert_eq!(entry.keywords, ["a", "b;c"]);
    }

    #[test]
    fn prefers_the_most_specific_locale_and_falls_back() {
        let text = "[Desktop Entry]\nName=Files\nName[de]=Dateien\nName[de_AT]=Dateien AT\nName[fr]=Fichiers\nComment=only default\n";
        assert_eq!(
            locale_candidates("de_AT.UTF-8@euro"),
            ["de_AT@euro", "de_AT", "de@euro", "de"]
        );
        let entry = parse(text, &locales());
        assert_eq!(entry.name, "Dateien AT");
        assert_eq!(entry.comment, "only default");
        assert_eq!(parse(text, &[]).name, "Files");
        assert!(locale_candidates("C").is_empty());
    }

    #[test]
    fn exec_handles_quotes_field_codes_and_escapes() {
        assert_eq!(
            argv(r#"env "A B=1" app --name=%c %f %U -- 100%%"#, "My App").unwrap(),
            ["env", "A B=1", "app", "--name=My App", "--", "100%"]
        );
        assert_eq!(
            argv(r#"sh -c "echo \"hi\" \$HOME""#, "x").unwrap(),
            ["sh", "-c", r#"echo "hi" $HOME"#]
        );
        assert_eq!(argv("app %i --flag", "x").unwrap(), ["app", "--flag"]);
    }

    #[test]
    fn malformed_exec_is_rejected() {
        assert!(argv("", "x").is_none());
        assert!(argv("   ", "x").is_none());
        assert!(argv("%f", "x").is_none());
        assert!(argv(r#"app "unterminated"#, "x").is_none());
    }
}
