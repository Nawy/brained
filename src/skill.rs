pub const SKILL_PATH: &str = ".claude/skills/brained/SKILL.md";

pub fn render_skill_md() -> String {
    r#"---
name: brained
description: Search indexed knowledge and research files via the brained MCP server.
---

# Brained: Knowledge & Research Search

This project has a `brained` MCP server indexing files into two categories:

- **knowledge** files: read-only reference material. Use them for information only.
  Never edit, delete, or otherwise modify a knowledge file.
- **research** files: working files. Safe to edit, add to, or remove during your work.

## Generated conversion files (`*.b.md`)

Any file ending in `.b.md` is generated automatically from a PDF, Word, PowerPoint, Excel, or
audio (mp3/m4a, via transcription) source file. Never edit a `.b.md` file directly — your changes
will be overwritten on the next scan. If the content is wrong, edit the original source file
instead; brained will reconvert it automatically the next time it scans.

## Searching

Call the `search_knowledge` tool with:
- `query`: a short, specific phrase or sentence (a few words to one sentence works
  best — this matches how the embedding model and chunk size are tuned). Prefer
  several short, targeted queries over one long, broad query.
- `type` (optional): `"knowledge"`, `"research"`, or `"both"` (default `"both"`) to
  scope the search.
- `top_k` (optional): how many results to return (default 5).

## Reading results

Each result is formatted as:

```
@<path> [knowledge|research]
<chunk text>
```

`@<path>` is the source file relative to the project root, tagged as `knowledge` or
`research`. If you need more context than the chunk shown, open the file at `<path>`
directly.

## Finding anomalies (`extract_anomaly`)

Sometimes the useful signal in a set of results isn't the consensus — it's the one
chunk that disagrees with everyone else (nine customers praise a feature, one hates
it for a specific reason; that one dissenting chunk is often worth surfacing, not
averaging away as noise).

Call `extract_anomaly` instead of `search_knowledge` when you're explicitly looking
for an outlier, contradiction, or dissenting opinion rather than doing a normal
lookup:
- `context_query`: same short phrase/sentence semantics as `search_knowledge`'s
  `query`.
- `type` (optional): `"knowledge"`, `"research"`, or `"both"` (default `"both"`).
- `top_k` (optional): how many candidates to compare (default 12 — higher than
  `search_knowledge`'s 5, since spotting an outlier requires enough of the
  surrounding consensus to contrast it against).

`extract_anomaly` returns the same `@<path> [knowledge|research]` chunk blocks as
`search_knowledge` — it does not pre-select the anomaly for you. Read through the
returned chunks yourself, find the one that diverges most strongly from the rest,
and report that specific chunk back with a short explanation of why it stands out —
not the full list, not a summary or average of all of them.
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentions_both_tools_and_all_sidecar_formats() {
        let md = render_skill_md();
        assert!(md.contains("search_knowledge"));
        assert!(md.contains("extract_anomaly"));
        assert!(md.contains(".b.md"));
        assert!(md.contains("PDF"));
        assert!(md.contains("Word"));
        assert!(md.contains("PowerPoint"));
        assert!(md.contains("Excel"));
        assert!(md.contains("mp3"));
    }

    #[test]
    fn starts_with_valid_frontmatter() {
        let md = render_skill_md();
        assert!(md.starts_with("---\nname: brained\n"));
    }
}
