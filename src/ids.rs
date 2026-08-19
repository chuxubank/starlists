use regex::Regex;

pub fn slugify(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let re = Regex::new(r"[^a-z0-9]+").unwrap();
    let slug = re.replace_all(&lower, "-");
    slug.trim_matches('-').to_string()
}

pub fn unique_slug(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|s| s == base) {
        return base.to_string();
    }
    for i in 2..1000 {
        let candidate = format!("{base}-{i}");
        if !taken.iter().any(|s| s == &candidate) {
            return candidate;
        }
    }
    format!("{base}-x")
}

pub fn new_plan_id(now: &str) -> String {
    let compact: String = now
        .chars()
        .filter(|c| c.is_ascii_digit())
        .take(14)
        .collect();
    format!("PLAN_{compact}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Web Mapping"), "web-mapping");
        assert_eq!(slugify("  Emacs / Lisp "), "emacs-lisp");
        assert_eq!(slugify("AI"), "ai");
    }

    #[test]
    fn unique_slug_suffixes() {
        let taken = vec!["web".into(), "web-2".into()];
        assert_eq!(unique_slug("web", &taken), "web-3");
        assert_eq!(unique_slug("gis", &taken), "gis");
    }
}
