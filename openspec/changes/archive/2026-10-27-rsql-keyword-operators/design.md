# Design

## Context

hawkBit 1.0.0 (`hawkbit-ql-jpa`, `RsqlParser`) delegates to `cz.jirutka.rsql:rsql-parser` 2.1.0. Its JavaCC grammar defines `AND : ";" | " and "`, `OR : "," | " or "`, skips spaces and tabs between tokens, and excludes the space from `UNRESERVED_STR`. The grammar has no `IGNORE_CASE`, so only lowercase keywords are operators. hawkBit's own `RsqlParserTest` covers `a == 6 or a == 7`, `a == 6 and a == 7 or a == '8'` and mixed grouping.

hawier's parser is a hand-written recursive descent that knows only the punctuation. An unquoted value runs to the next `, ; ( )` and is trimmed, so the whole tail of a keyword query is swallowed into the first value. That is why the bug answers 200 with total 0 instead of 400.

## Goals / Non-Goals

**Goals:**
- A keyword query means what it means in hawkBit.
- A query hawkBit refuses because of a stray word, such as `AND` or `&&`, is refused here too, instead of matching nothing.

**Non-Goals:**
- Whitespace around comparison operators (`a == 6`), which hawkBit accepts and hawier refuses with 400. That is a remaining divergence, below.
- hawkBit's `=is=`/`=not=` aliases and its refusal of `= < > ! ~` inside unquoted values. Neither causes a silent wrong answer.

## Decisions

**Unquoted values end at whitespace.** Adding only the two lowercase keywords would fix the reported query, but `a==1 AND b==2` would still read as one comparison with the value `1 AND b==2`, and still answer 200 with total 0. Ending the value at whitespace, as rsql-parser does, turns every such typo into trailing input and 400. Alternative considered: recognise `and`/`or` case-insensitively. Rejected, because it would accept queries hawkBit refuses and still leave `&&` silent.

**Keywords need whitespace on both sides.** A value or selector may contain `and` or `or` (`name==sandor`, `name==or`), so a keyword is recognised only after at least one whitespace character and before another. rsql-parser behaves the same, since `UNRESERVED_STR` absorbs letters that touch a value.

**Whitespace is insignificant around separators and parentheses.** rsql-parser skips it everywhere. Ignoring it around `, ; ( )` is needed anyway once a value ends at whitespace (`(a==1 or b==2) and c==3`). Whitespace between selector, operator and value is still refused, which keeps the existing "whitespace around an operator" rule.

**The console quotes its search needle.** The needle is typed text. With values ending at whitespace, `name==*foo bar*` would become 400, and a needle containing ` and ` would turn into a query. The needle is already stripped of `"`, `'` and `\`, so wrapping it in `"..."` cannot break out of the quote, and `*` inside quotes is still a wildcard in both hawier and hawkBit. Every other place the console assembles RSQL either quotes the value (`filtered_list_href`, `row_href`) or inserts an id or an enum name. Special-casing the words `and`/`or` in the sanitizer is not needed, and would change what the user searched for.

## Risks / Trade-offs

- [A filter stored by hawier with an unquoted spaced value now fails when it is evaluated] → hawkBit never accepted such a filter, and hawier is pre-production. Target filters and rollouts re-validate the query when it is created, so new ones are refused up front.
- [Someone used inner spaces in a hand-written `q`] → the 400 names the position. The manual's quoting section now says to quote such values.

## Divergences

- hawkBit accepts whitespace around comparison operators (`a == 6`); hawier answers 400. A delta to "Grammar and operators" through its own change would close it.
