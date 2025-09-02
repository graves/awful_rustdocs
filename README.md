# Awful Rustdocs

[![Crates.io](https://img.shields.io/crates/v/awful_rustdocs.svg)](https://crates.io/crates/awful_rustdocs)

Awful Rustdocs is a CLI that generates or improves Rustdoc for your codebase by harvesting symbols with a tiny Nu (Nushell) script, enriching per-item context using ast-grep, and asking Awful Jade to draft concise, high-quality docs. It can write the docs back into your source files in the correct location while preserving attributes like #[derive(...)], #[serde(...)], etc.

It supports:
- Functions (fn): full context (signature, callers, referenced symbols, calls-in-span).
- Structs: short top-level struct summary (above attributes) plus inline field comments generated from the struct body and references in the codebase (via a structured response_format).
- Selective processing via --only (case-sensitive, matches simple name or fully qualified path).
- Safe, idempotent insertion with --overwrite off by default.

## How it works

1. Harvest items with rust-ast.nu

You provide (or use the default) Nu script rust_ast.nu that emits a list of items (at least fn and struct) with fields like:
- `kind` ("fn", "struct")
- `file`, `fqpath`, `name`, `visibility`
- `signature` (the item line the user would see)
- `span` (start/end line/byte)
- `doc` (existing comment, if any)
- `body_text` (item body where present)
- `callers` (if your pipeline includes it)

These rows are read by Awful Rustdoc and grouped per file.

> Note: Field comments are not read from the Nu output. For structs, the tool asks the LLM once with the struct body and references and receives inline field docs in a structured payload (see below). The patcher then inserts a /// line immediately above each field, aligned to the field’s indentation and placed above any field attributes.

2. Augment context with [ast-grep](https://ast-grep.github.io/guide/quick-start.html)

For functions (unless disabled):
-  Call sites inside the function body (plain, qualified, method).
-  Qualified paths `(A::B, A::<T>::B, A::{...})` discovered by pattern queries.

These additional hints help the LLM write better docs.

3. Pick the right template

Two templates are loaded from your Awful Jade template directory:
- `--template` (default: rustdoc_fn): for functions
- `--struct-template` (default: rustdoc_struct): for structs + fields

The struct template is expected to specify a response_format JSON schema. The model returns structured JSON that contains:
- A doc for the struct (short summary, no sections).
- A list of fields[] (name + rustdoc text) to be inserted inline.

You have full control over wording and constraints in those templates.

4. Ask Awful Jade

For each item:
- Build a rich, markdown prompt with identity, existing docs, and context.
- For functions, the model returns a plain /// block.
- For structs, the model returns JSON conforming to your response_format schema (see next section), from which the program extracts:
- The top struct doc (converted into a strict /// block),
- The per-field docs (each as a single /// line or short block).

All model output is passed through sanitizers:
- `strip_wrappers_and_fences_strict`: safely removes wrapper tokens (e.g., ANSWER:) only when they appear at line starts and outside fences; it won’t nuke inline code examples.
- `sanitize_llm_doc`: converts any prose to strict /// lines; trims double-blanks; balances fences; removes a leading empty /// if present.

5. Patch files safely

For each edit, the patcher:
- Finds the right insertion window:
- Functions: directly above the function signature (consumes an immediately preceding blank if present).
- Structs: above attributes (e.g., #[derive], #[serde]) so the doc block sits at the very top, preserving the attribute group below.
- Fields: immediately above the field, above any field attributes; aligned to field indentation.
- Skips any item that already has docs unless --overwrite is set.
- Applies all edits from the bottom up to avoid shifting line offsets.
- Writes artifacts to `target/llm_rustdocs/docs.json`.

JSON schema for struct `response_format`

Your `--struct-template` should define a strict response_format similar to:
```yaml
response_format:
  name: rustdoc_struct_v1
  strict: true
  description: Rust struct docs plus per-field inline docs.
  schema:
    type: object
    properties:
      doc:
        type: string
        description: A short 1–2 sentence struct summary. No headings, no lists. Pure prose.
      fields:
        type: array
        description: Inline field docs. Ignored if field name not found in struct body.
        items:
          type: object
          properties:
            name:
              type: string
              description: Field identifier exactly as it appears in the struct.
            doc:
              type: string
              description: The rustdoc content for this field (one or a few `///` lines of prose).
          required: [name, doc]
    required: [doc, fields]
```
Your system / pre/post user messages should instruct the model to:
- Return only JSON conforming to that schema (no prose).
- Keep the struct summary short and field docs concise.
- Avoid restating types unless it clarifies semantics (units, invariants, ranges).
- Preserve exact field names.

The program will:
- Convert doc to a strict /// block placed above any attribute lines that decorate the struct.
- Insert each fields[].doc as /// immediately above the corresponding field, with the field’s current indentation and preserving any field attributes below the doc.

## Installation

Requirements:
- Rust (stable)
- Nushell (nu) to run `rust-ast.nu`
- ast-grep (CLI) for call/path discovery in functions
- Awful Jade config & templates (see `--config`, `--template`, `--struct-template`)

0. Prerequisites

You’ll need:
- Rust & Cargo (stable)
	- macOS/Linux: https://rustup.rs
	- Windows (PowerShell): `winget install Rustlang.Rustup`
- Nushell (nu)
	- macOS: `brew install nushell`
	- Linux: `snap install nushell --classic` or `cargo install nu`
	- Windows: `winget install nushell`
- ast-grep (CLI)
	- macOS: `brew install ast-grep`
	- Linux: download release from GitHub or cargo install `ast-grep-cli`
	- Windows: scoop install `ast-grep` or download release

Tip: Verify tools after install:
```shell
cargo --version
nu --version
ast-grep --version
```

1. Install the CLI from crates.io
```shell
cargo install awful_rustdoc
```
This will put the awful_rustdoc binary in `~/.cargo/bin` (ensure that’s on your `PATH`).

2. Set up Awful Jade config & template directories

Your program resolves config and templates under the app config directory. Use these conventional locations:
- macOS/Linux: `~/.config/awful_jade/`
- Windows: `%APPDATA%\awful_jade\`

Create the folders (macOS/Linux shown—adapt paths on Windows):
```shell
mkdir -p ~/.config/awful_jade/templates
```
If you already have a config (e.g. config.yaml) for Awful Jade, place it at:
- macOS/Linux: `~/.config/awful_jade/config.yaml`
- Windows: `%APPDATA%\awful_jade\config.yaml`

3. Get the latest templates from `graves/awful_rustdocs`

#### Option A: Git clone then copy

# macOS/Linux
```shell
git clone https://github.com/graves/awful_rustdocs /tmp/awful_rustdocs
cp -R /tmp/awful_rustdocs/templates/* ~/.config/awful_jade/templates/
```
#### Option B: Download specific files (example)

If you only want the two defaults your program uses:
- `rustdoc_fn.yaml`
- `rustdoc_struct.yaml`

```shell
curl -L \
  https://raw.githubusercontent.com/graves/awful_rustdocs/HEAD/templates/rustdoc_fn.yaml \
  -o ~/.config/awful_jade/templates/rustdoc_fn.yaml

curl -L \
  https://raw.githubusercontent.com/graves/awful_rustdocs/HEAD/templates/rustdoc_struct.yaml \
  -o ~/.config/awful_jade/templates/rustdoc_struct.yaml
```
These defaults match your flags `--template rustdoc_fn` and `--struct-template rustdoc_struct`. If you rename them, pass the new names via flags.

4. Get `rust_ast.nu` from `graves/nu_rust_ast`

Your binary defaults to `--script rust_ast.nu` (looked up in the current working directory). Place it in your repo root or wherever you’ll run the command from.
```shell
curl -L \
  https://raw.githubusercontent.com/graves/nu_rust_ast/HEAD/rust_ast.nu \
  -o ./rust_ast.nu
```
(Or `git clone https://github.com/graves/nu_rust_ast` and copy rust_ast.nu from there.)

5. Quick smoke test

From your project root (where rust_ast.nu now lives):
```shell
awful_rustdoc --limit 1
```
You should see `target/llm_rustdocs/docs.jso`n produced. This confirms the harvesting pipeline and template loads are working. Nothing is written to your source files unless you add --write.
If you hit any bumps, ping me with the error output and the snippet—you’ve already got a solid pipeline, so it’s typically a small path/filename or template mismatch.

## Command-line usage

```shell
λ awful_rustdocs --help
Generate rustdocs for functions and structs using Awful Jade + rust_ast.nu

Usage: awful_rustdocs [OPTIONS] [TARGETS]...

Arguments:
  [TARGETS]...  Paths (files/dirs) to analyze (default: ".")

Options:
      --script <SCRIPT>
          Path to your rust_ast.nu (the Nu script you shared)", [default: rust_ast.nu]
      --write
          Write docs directly into source files (prepending ///)",
      --overwrite
          Overwrite existing rustdoc if present (default: false; only fills missing)",
      --session <SESSION>
          Session name for Awful Jade; if set, enables memory/session DB
      --limit <LIMIT>
          Limit the number of items processed (for testing)",
      --no-calls
          Skip per-function ast-grep call-site analysis
      --no-paths
          Skip per-function qualified path analysis
      --fn-template <FN_TEMPLATE>
          Template for functions (expects response_format JSON)", [default: rustdoc_fn]
      --struct-template <STRUCT_TEMPLATE>
          Template for structs+fields (expects response_format JSON)", [default: rustdoc_struct]
      --config <CONFIG>
          Awful Jade config file name under the app config dir (e.g. "config.yaml") [default: config.yaml]
      --only <SYMBOL>...
          Only generate docs for these symbols (case-sensitive)
  -h, --help
          Print help
```

## Examples

1. Dry-run over the whole repo (no file changes)
```shell
awful_rustdoc
```
Artifacts go to t`arget/llm_rustdocs/docs.json`.

2. Write missing function docs across src/
```shell
awful_rustdoc src --write
```

3. Overwrite all existing function docs
```shell
awful_rustdoc --write --overwrite
```

4. Document a single function by name (case-sensitive)
```shell
awful_rustdoc --only do_work --write
```

5. Document a specific function by fully-qualified path
```shell
awful_rustdoc --only my_crate::utils::do_work --write
```

6. Document one struct and its fields only
```shell
awful_rustdoc --only my_crate::types::Config --write
```

The tool will:
- Ask the struct template for JSON (doc + fields[]),
- Place the struct summary above its attributes,
- Insert /// comments right above each field found in the body, aligned with the field’s indentation.


## Insertion rules & safety
- Struct docs: Placed above the attribute block (#[derive], #[serde], …) so attributes remain directly attached to the struct item. One optional blank line may be kept above the attribute block for readability.
- Field docs: Placed above the field, and above any field attributes. Indentation matches the field line so the comments are visually aligned.
- Function docs: Inserted directly above the fn signature; if there’s a single blank line immediately above, it’s absorbed into the doc block.
- Overwrite behavior:
- With --overwrite off (default), items that already have docs are skipped.
- With --overwrite on, only the existing doc lines are replaced; attributes remain intact.
- Bottom-up edits: All edits per file are sorted by descending byte offset, so earlier patches don’t shift the spans of later ones.

## Output artifacts
- `target/llm_rustdocs/docs.json` — a structured dump of everything generated:
```json
[
  {
    "kind": "fn" | "struct",
    "fqpath": "...",
    "file": "src/...rs",
    "start_line": 123,
    "end_line": 167,
    "signature": "pub fn ...",
    "callers": [...],
    "referenced_symbols": [...],
    "llm_doc": "/// lines...\n/// ...",
    "had_existing_doc": false
  }
]
```

## Template tips
- Function template (rustdoc_fn)
Have -  assistant return only a /// block (no backticks, no prose outside). Encourage:
- 1–2 sentence summary,
- Optional sections: Parameters:, Returns:, Errors:, Safety:, Notes:, Examples:,
- Doc-test friendly examples (no fenced code unless necessary),
- Avoid leading empty ///.
- Struct template (rustdoc_struct)
Use t- response_format JSON schema shown above. In your instructions:
- Provide the full struct body and a list of functions that use the struct (to help write the field docs).
- Ask for concise prose (doc) and a fields[] array.
- Tell the model not to duplicate the type, unless needed for semantics (units, invariants, ranges).
- Ask it to return only JSON.

## Behavior that prevents mangling
- Wrapper token stripping is only applied at line starts and outside fences, and is skipped if the payload already looks like a rustdoc block.
- Sanitization:
- Collapses repeated blank lines into single ///,
- Ensures every line starts with ///,
- Removes a single leading blank /// if present,
- Balances code fences (/// ```rust … `/// ```),
- Leaves inline code alone.

## Troubleshooting
- Attributes were moved/removed:
	- Struct and field patchers only target doc lines and are designed to leave `#[...]` blocks untouched. If you see attributes removed, verify your template didn’t emit attribute-like text and that you’re not running another formatter concurrently.
- A single struct looks garbled:
	-This usually means the model emitted stray characters or mixed prose when it should have returned only JSON (for structs). Tighten the response_format constraints and system rules; consider enabling strict: true.
- No fields were commented:
	- Ensure your struct template returns the fields[] array with exact field names as they appear in the code. The patcher only inserts for names it can find.
- Function analysis is slow:
	- Try -`-no-calls` and/or `--no-paths` to skip `ast-grep` passes.

## Limitations
- The tool assumes reasonably idiomatic Rust formatting for matching signatures and fields.
- Exotic macro-expanded items may not be discoverable or patchable.
- The field matcher is heuristic; very complex multi-line field definitions may require tweaks.

## License

CC0-1.0
