# D-286 Repository-wide proofs fail closed on directory enumeration

Status: Accepted (2026-09-05), amends D-124 and D-215; advances PW-525

**Context.** The unsafe, citation, page-size, target-source, Markdown/link,
host-crate, model, vendored-directory and inline-command inventories opened
their directories but then skipped some enumeration, classification or
metadata errors. An unreadable entry could therefore disappear from an “all
files” claim without making the check fail.

**Decision.** One `xtask::paths` helper collects every `DirEntry` result and
resolves every file type before a repository proof interprets any path.
Opening, enumerating or classifying the directory returns an error with its
path identity; redirected and special entries are refused. All affected
inventories use that helper. Filtering begins only after the complete,
classified listing exists, and a crate manifest distinguishes absence from
every other metadata failure.

**Alternatives and evidence.** Treating an unreadable entry as out of scope was
rejected because unreadability is not evidence that it contains no source,
crate, document or vendored authority. Repeating `expect` in each iterator was
rejected because the fail-closed rule would remain easy to omit. Injected
enumeration and classification errors now fail their collectors, and a pure
classification witness rejects redirected and special entries. Replacing
either collection with `filter_map`, accepting either rejected kind, turning
unreadable citation input into empty text, or treating optional-source metadata
failure as absence made its exact witness red before restoration. Those six
defects remain mutations 1,295 through 1,300. Full API audit and CI remain
PW-525's integrated proof.
