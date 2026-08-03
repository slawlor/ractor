use std::cell::Cell;

use ractor::MessagingErr;

fn assert_sync<T: Sync>() {}

fn main() {
    assert_sync::<MessagingErr<Cell<u8>>>();
}
