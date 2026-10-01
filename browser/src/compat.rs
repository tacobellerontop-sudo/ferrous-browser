//! Site interventions: small, targeted scripts that work around a specific
//! site's dependence on a web feature Servo does not have yet.
//!
//! This is the same mechanism Firefox ships as "webcompat interventions": rather
//! than faking a feature for the whole web (and risking breaking the many sites
//! that feature-detect it correctly), each fix names the sites it is for, why it
//! is needed, and what upstream change would let it be deleted.
//!
//! Every script guards itself on `location.hostname`, so one set of user
//! scripts can be attached to every tab and is inert everywhere else.

/// One intervention.
pub struct Intervention {
    /// Hosts the script runs on; a host also covers its subdomains.
    pub hosts: &'static [&'static str],
    /// What is broken without it, for logs and for whoever removes it.
    pub reason: &'static str,
    /// The script body. Wrapped in the host guard by [`script`].
    pub body: &'static str,
}

/// All interventions. Each one should name the Servo issue whose fix makes it
/// unnecessary.
pub const INTERVENTIONS: &[Intervention] = &[Intervention {
    hosts: &["youtube.com"],
    reason: "YouTube search results and pages render blank: Servo has no document.all \
             (servo/servo#7396), so polymer-resin's falsy-value guard \
             `!v && v !== document.all` fails for undefined and replaces it with a truthy \
             placeholder, which hides bound containers.",
    // `document.all` is the one falsy object in JavaScript, and that cannot be
    // reproduced in script. What resin needs is a falsy value that is not
    // `undefined` or `null`, so `undefined !== document.all` holds as it does in
    // every other browser. NaN is falsy and unequal to everything, which makes
    // all three of resin's checks behave exactly as in Chrome. YouTube's code
    // uses document.all nowhere else.
    body: "if (!('all' in Document.prototype) && !('all' in document)) { \
           Object.defineProperty(Document.prototype, 'all', { get: function () { return NaN; }, configurable: true }); }",
}];

/// The script for `intervention`, guarded so it only runs on its hosts.
pub fn script(intervention: &Intervention) -> String {
    let hosts = intervention
        .hosts
        .iter()
        .map(|h| format!("{h:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "(function () {{ var h = location.hostname; \
         if (![{hosts}].some(function (d) {{ return h === d || h.endsWith('.' + d); }})) return; \
         {} }})();",
        intervention.body
    )
}

/// Whether `host` is covered by `intervention`; the Rust twin of the guard in
/// [`script`], which the tests check it against.
#[cfg(test)]
pub fn applies_to(intervention: &Intervention, host: &str) -> bool {
    intervention
        .hosts
        .iter()
        .any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_intervention_covers_youtube_and_its_subdomains_only() {
        let yt = &INTERVENTIONS[0];
        assert!(applies_to(yt, "youtube.com"));
        assert!(applies_to(yt, "www.youtube.com"));
        assert!(applies_to(yt, "music.youtube.com"));
        assert!(!applies_to(yt, "notyoutube.com"));
        assert!(!applies_to(yt, "youtube.com.evil.example"));
    }

    #[test]
    fn scripts_are_host_guarded_and_wrapped() {
        for i in INTERVENTIONS {
            let s = script(i);
            assert!(s.starts_with("(function () {"), "{s}");
            assert!(s.contains("location.hostname"));
            for host in i.hosts {
                assert!(s.contains(&format!("\"{host}\"")));
            }
            assert!(s.ends_with("})();"));
        }
    }

    #[test]
    fn every_intervention_explains_itself() {
        for i in INTERVENTIONS {
            assert!(!i.hosts.is_empty());
            assert!(i.reason.len() > 40, "say what breaks and why");
        }
    }
}
