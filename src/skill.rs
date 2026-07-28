pub const SKILL_PATH: &str = ".claude/skills/brained/SKILL.md";

pub fn render_skill_md() -> String {
    r#"---
name: brained
description: Search indexed knowledge and work files via the brained MCP server.
---

# Brained: Knowledge & Work Search

This project has a `brained` MCP server indexing files under four mandatory folders,
crossing two domains (**business**, **tech**) with two kinds (**knowledge**, **work**):

- `business/knowledge/` and `tech/knowledge/`: read-only reference material (past problem
  solutions, resolved issues, accumulated business/technical info). Use them for information
  only. Never edit, delete, or otherwise modify a knowledge file.
- `business/work/`: active business scratch space — WIP analysis, notes, TODOs. Safe to edit,
  add to, or remove during your work.
- `tech/work/`: active engineering scratch space — WIP debugging notes, design docs, TODOs.
  Also safe to edit, but note it is **not searchable** (see Searching below) — it exists for
  writing, not lookup.

Promotion from a `work/` file to a `knowledge/` file is a manual, human-only step outside
this MCP server — there is no tool for it. Don't attempt to move or rewrite a file into a
`knowledge/` folder yourself.

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
- `domain` (optional): `"business"`, `"tech"`, or `"both"` (default `"both"`).
- `kind` (optional): `"knowledge"`, `"work"`, or `"both"` (default `"both"`).
- `top_k` (optional): how many results to return (default 5).

`business/knowledge`, `tech/knowledge`, and `business/work` are all searchable. `tech/work`
is never scanned or embedded — it will never appear in search results no matter which
`domain`/`kind` filter you use, even though it's a real, writable folder (see Writing work
files below).

## Reading results

Each result is formatted as:

```
@<path> [<domain>/<kind>]
<chunk text>
```

`@<path>` is the source file relative to the project root, tagged with its domain and kind
(e.g. `business/knowledge`, `tech/work`). If you need more context than the chunk shown,
open the file at `<path>` directly.

## Writing work files

Call `write_work_file` to create or overwrite a file under `<domain>/work/`:
- `domain`: `"business"` or `"tech"`.
- `path`: relative to `<domain>/work/`, e.g. `"debug-notes/2026-07-27.md"`. Never targets
  `knowledge/` — the tool rejects any path escaping `work/` or containing a `knowledge`
  component.
- `content`: the file's full new content.

`write_work_file` requires holding that domain's lock first — call `locktech`/`lockbusiness`
before it, even if you believe no one else is using the domain right now.

## Locking

Other Claude Code sessions may be working against this same project at the same time. Before
a planned sequence of `write_work_file` calls (or a read you need to stay stable across
several tool calls), acquire the relevant domain's lock:

- `locktech()` / `lockbusiness()`: try-lock, not blocking. Returns whether you now hold it —
  `false` means another session already holds it.
- `unlocktech()` / `unlockbusiness()`: release your own hold when you're done.

If a lock call returns "locked by another session":
- If you wanted it to make a **planned write**, silently retry the same lock call roughly
  every 30 seconds until it succeeds — don't ask the human each time.
- If you wanted it to protect a **read** (so files won't change mid-read), ask the human
  whether to proceed read-only anyway or wait for the lock, rather than retrying silently.

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
- `domain` (optional): `"business"`, `"tech"`, or `"both"` (default `"both"`).
- `kind` (optional): `"knowledge"`, `"work"`, or `"both"` (default `"both"`).
- `top_k` (optional): how many candidates to compare (default 12 — higher than
  `search_knowledge`'s 5, since spotting an outlier requires enough of the
  surrounding consensus to contrast it against).

`extract_anomaly` returns the same `@<path> [<domain>/<kind>]` chunk blocks as
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
    fn mentions_domain_folders_and_work_file_writing() {
        let md = render_skill_md();
        assert!(md.contains("business/knowledge"));
        assert!(md.contains("business/work"));
        assert!(md.contains("tech/knowledge"));
        assert!(md.contains("tech/work"));
        assert!(md.contains("write_work_file"));
    }

    #[test]
    fn mentions_lock_tools_and_retry_guidance() {
        let md = render_skill_md();
        assert!(md.contains("locktech"));
        assert!(md.contains("unlocktech"));
        assert!(md.contains("lockbusiness"));
        assert!(md.contains("unlockbusiness"));
        assert!(md.contains("30 seconds"));
    }

    #[test]
    fn starts_with_valid_frontmatter() {
        let md = render_skill_md();
        assert!(md.starts_with("---\nname: brained\n"));
    }
}
