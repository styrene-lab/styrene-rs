# Extract Styrene UI Repository Tasks

## 1. Extraction Preparation
<!-- specs: gui-repository/spec -->

- [ ] 1.1 Confirm `styrene-lab/styrene-ui` ownership, visibility, licensing, maintainers, and branch protections
- [x] 1.2 Record the immutable `styrene-rs` source revision and extraction path set
- [ ] 1.3 Verify `styrene-dx` uses only public shared client, session, IPC, and runner boundaries
- [ ] 1.4 Define desktop acceptance commands and the authority-switch rollback condition

## 2. History-Preserving Extraction
<!-- specs: gui-repository/spec -->

- [x] 2.1 Create the new repository without modifying either protected Styrene checkout
- [x] 2.2 Extract `crates/apps/styrene-dx` history from an isolated temporary clone
- [x] 2.3 Add provenance documentation with source repository, revision, paths, and extraction method
- [x] 2.4 Verify representative files retain history or explicit source mappings

## 3. Independent Workspace
<!-- specs: gui-repository/spec -->

- [x] 3.1 Establish the `styrene-ui` workspace, toolchain, licenses, repository guidance, and generated-artifact exclusions
- [x] 3.2 Pin application-facing `styrene-rs` crates to one immutable full revision
- [x] 3.3 Remove assumptions about a sibling `styrene-rs` checkout and local process paths
- [x] 3.4 Separate shared presentation state, Dioxus components, and desktop launcher without changing behavior
- [x] 3.5 Keep generated native output excluded and disposable after later mobile hosts replace the original empty-directory reservation

## 4. Desktop And Lab Validation
<!-- specs: gui-repository/spec -->

- [ ] 4.1 Run formatting, warning-denied Clippy, unit, reducer, and component tests
- [ ] 4.2 Run desktop Fixture, Live-failure, and Embedded smoke scenarios
- [x] 4.3 Verify Fixture opens no daemon process or external network interface
- [x] 4.4 Verify Lab uses the declared runner boundary and retains bounded cancellation and cleanup
- [x] 4.5 Verify a clean checkout resolves without local path dependencies

## 5. Authority Switch
<!-- specs: gui-repository/spec -->

- [x] 5.1 Publish the validated GUI revision and record its tested `styrene-rs` revision
- [x] 5.2 Remove the maintained `styrene-dx` source from `styrene-rs` in a separate commit
- [x] 5.3 Add repository pointers and coordinated compatibility guidance to both repositories
- [x] 5.4 Verify `styrene-rs` TUI, workspace, documentation, and release checks after removal
- [x] 5.5 Confirm only `styrene-ui` accepts subsequent Dioxus application changes
