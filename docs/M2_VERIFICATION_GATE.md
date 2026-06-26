# M2 Verification Gate — result

**Question (manifest Decision 1 / §4.2):** do the surviving UNL corpus UCLs
resolve against WordNet 3.1 synset offsets, so we can adopt WordNet offsets
*verbatim* as our UCL ids and get corpus interop for free?

**Answer: No — not against WordNet 3.1.** The gate **fails**. Evidence below.
The manifest's documented fallback applies.

## What the corpus UCLs actually are

Source: the AESOP corpus from the surviving `unlarchive.org` mirror
(`export_corpus.php?project=aa1`), fetched by `cargo run -p xtask -- fetch-aesop`
(78 sentences, 6 languages, gitignored — no explicit open licence).

Every numeric UW id in the corpus has the shape **`<pos><8-digit-offset>`**:

| Leading digit | POS  | Count (all 6 langs) |
|---------------|------|---------------------|
| `1`           | noun | 84                  |
| `2`           | verb | 66                  |
| `3`           | adj  | 30                  |
| `4`           | adv  | 6                   |

No ids start with 5–9. So the corpus convention is:

```
UCL = pos_digit * 100_000_000 + wordnet_offset      (pos: 1=n, 2=v, 3=a, 4=r)
```

This **confirms a POS tag must be encoded in the id** — a bare offset is
ambiguous across parts of speech (we independently found 95 noun/verb offset
collisions in WordNet 3.1, e.g. `1740` = noun *entity* = verb *breathe*).

## But the offsets are WordNet 3.0, not 3.1

The 8-digit offsets do **not** match WordNet 3.1. Every sampled id is a
near-miss — consistent with the 3.0 → 3.1 synset renumbering:

| Word (corpus)      | Corpus id   | → offset   | WordNet **3.1** offset | Δ      |
|--------------------|-------------|------------|------------------------|--------|
| hare (n)           | `102326432` | `02326432` | `02329084`             | +2652  |
| tortoise (n)       | `101670092` | `01670092` | `01672733`             | +2641  |
| ridicule (v)       | `200851933` | `00851933` | `00853615`             | +1682  |

None of the corpus offsets exist at those positions in WordNet 3.1's data
files. The original UNLKB was therefore seeded from **WordNet 3.0** (or earlier),
not 3.1.

## Consequence for the design

The manifest's Decision 1 rationale — "adopt WordNet 3.1 offsets verbatim →
partial interop with surviving UNL corpora for free" — **does not hold**. Adopting
3.1 offsets verbatim buys *no* corpus interop, because the corpus speaks 3.0.

Options (for the user to weigh):

1. **Seed from WordNet 3.0 instead of 3.1**, using the corpus's own id layout
   (`pos*1e8 + offset`, pos 1–4). This is the *only* path to direct, free corpus
   interop. Cost: 3.0 is older; 3.1 is the last Princeton release. `xtask` would
   fetch `WordNet-3.0.tar.gz` instead.
2. **Take the manifest's stated fallback** (Decision 1): keep WordNet 3.1 as the
   seed, store the WordNet offset as *metadata*, mint our own ids. Add a
   separately-built **3.0→3.1 offset map** (or a lemma+sense-key remap) if/when
   corpus interop is actually needed. Corpus stays usable as NL/UNL fixtures via
   lemma resolution even without numeric id equality.
3. **Hybrid:** seed 3.1 but adopt the corpus id *layout* (`pos*1e8 + offset`) so
   our ids are at least structurally UNL-native, accepting that the numeric
   values differ from the 3.0 corpus.

## Current code state

`WordNetKb` (Rev 1) seeds from **3.1** and uses a POS-blocked id scheme at base
`1e9` (noun `0`, verb `1e9`, adj `2e9`, adv `3e9`) — internally correct and
collision-free, but **not** aligned to the corpus's `pos*1e8 + offset` layout and
**not** offset-compatible with the 3.0 corpus. Changing the seed version and/or
id base is the open decision above; it is isolated to `crates/unl-kb/src/wordnet.rs`
(the `Pos::block`/`ucl`/`from_ucl` functions) plus the `xtask` download URL.

`parse_legacy_document` (the `[D]/[S]/{org}/{unl}` envelope this corpus uses) is
still unimplemented; the corpus shows it needs node-id suffixes (`102326432:73`),
relation scopes (`and:01(...)`), scope-reference args (`:01.@def`), null-with-id
(`00:3F.@1`), and unquoted multiword headwords (`take a nap.@past`).
