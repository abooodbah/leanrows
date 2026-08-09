## Summary

Describe the problem and the focused change that addresses it.

## Verification

- [ ] `cargo +1.97.1 fmt --all -- --check`
- [ ] `cargo +1.97.1 test --workspace --all-targets --locked`
- [ ] `cargo +1.97.1 clippy --workspace --all-targets --locked -- -D warnings`
- [ ] Relevant native, document, QA, or packaging checks

List the exact additional commands and results:

```text

```

## Product and safety checks

- [ ] Source files remain read-only.
- [ ] File-size-dependent allocations and outputs have explicit bounds.
- [ ] UI callbacks do not perform file I/O or wait on background work.
- [ ] Malformed input is visible and is not silently repaired.
- [ ] Public documentation and tests match observable behavior.
- [ ] No confidential fixture data or credentials are included.

## Evidence

For performance or memory changes, include the fixture description and hash,
host details, raw measurements, and comparison method. Leave this section as
not applicable when the change does not make a performance or memory claim.
