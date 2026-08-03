// Copyright (c) Sean Lawlor
//
// This source code is licensed under both the MIT license found in the
// LICENSE-MIT file in the root directory of this source tree.

#[test]
fn messaging_error_only_inherits_sync_from_its_message() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/error_traits/messaging_err_non_sync.rs");
}
