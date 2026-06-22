//! `<available_skills>` XML block for the system prompt.
//!
//! Renders one `<skill>` element per skill with its name and description,
//! both XML-escaped. The block is meant to be appended to the system prompt
//! so the model knows which skills it can invoke via the `skill` tool.

/// Build the `<available_skills>` block from a slice of skill references.
/// Renders one `<skill>` element per skill with its name and description,
/// XML-escaped.
pub fn build_xml(skills: &[&mew_skills::Skill]) -> String {
    let mut buf = String::from("<available_skills>\n");
    for skill in skills {
        buf.push_str(&format!(
            "  <skill>\n    <name>{}</name>\n    <description>{}</description>\n  </skill>\n",
            escape_xml(&skill.name),
            escape_xml(&skill.description),
        ));
    }
    buf.push_str("</available_skills>\n");
    buf
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mew_skills::Skill;

    fn skill(name: &str, desc: &str) -> Skill {
        Skill {
            name: name.into(),
            description: desc.into(),
            body: String::new(),
            path: std::path::PathBuf::from("(test)"),
        }
    }

    #[test]
    fn test_build_xml_empty() {
        let xml = build_xml(&[]);
        assert!(xml.contains("<available_skills>"));
        assert!(xml.contains("</available_skills>"));
    }

    #[test]
    fn test_build_xml_renders_one_block_per_skill() {
        let s1 = skill("alpha", "the first one");
        let s2 = skill("beta", "the second one");
        let xml = build_xml(&[&s1, &s2]);
        assert!(xml.contains("<name>alpha</name>"));
        assert!(xml.contains("<description>the first one</description>"));
        assert!(xml.contains("<name>beta</name>"));
        assert!(xml.contains("<description>the second one</description>"));
    }

    #[test]
    fn test_build_xml_escapes_special_chars() {
        let s = skill("name", "has <html> & \"quotes\"");
        let xml = build_xml(&[&s]);
        assert!(xml.contains("&lt;html&gt;"));
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&quot;quotes&quot;"));
        // The raw special chars must not appear inside the description.
        assert!(!xml.contains("<html>"));
    }
}
