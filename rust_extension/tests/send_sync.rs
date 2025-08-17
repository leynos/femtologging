//! Send/Sync guarantees for core types.

use _femtologging_rs::{
    ConfigBuilder, FemtoHandler, FemtoLogger, FemtoStreamHandler, FileHandlerBuilder,
    FormatterBuilder, LoggerConfigBuilder, StreamHandlerBuilder,
};
use static_assertions::assert_impl_all;

#[test]
fn builders_are_send_sync() {
    assert_impl_all!(ConfigBuilder: Send, Sync);
    assert_impl_all!(FormatterBuilder: Send, Sync);
    assert_impl_all!(LoggerConfigBuilder: Send, Sync);
    assert_impl_all!(StreamHandlerBuilder: Send, Sync);
    assert_impl_all!(FileHandlerBuilder: Send, Sync);
}

#[test]
fn components_are_send_sync() {
    assert_impl_all!(FemtoStreamHandler: Send, Sync);
    assert_impl_all!(FemtoLogger: Send, Sync);
    assert_impl_all!(FemtoHandler: Send, Sync);
}
