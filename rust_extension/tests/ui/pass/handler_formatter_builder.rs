//! Compile-pass coverage for the public handler and formatter builder APIs.

use _femtologging_rs::{
    ConfigBuilder, FileHandlerBuilder, FormatterBuilder, HandlerBuilder, HTTPHandlerBuilder,
    LoggerConfigBuilder, StreamHandlerBuilder,
};

fn main() {
    let _formatter = FormatterBuilder::new()
        .with_format("%(message)s")
        .with_datefmt("%Y");
    let file = FileHandlerBuilder::new("compile.log").with_formatter("%(message)s");
    let http: HandlerBuilder = HTTPHandlerBuilder::new()
        .with_url("https://example.invalid/logs")
        .with_filters(["context"])
        .into();
    assert!(matches!(&http, HandlerBuilder::Http(_)));
    let stream = StreamHandlerBuilder::stderr().with_filters(["audit"]);

    let _config = ConfigBuilder::new()
        .with_handler("file", file)
        .with_handler("remote", http)
        .with_handler("stderr", stream)
        .with_root_logger(
            LoggerConfigBuilder::new().with_handlers(["file", "remote", "stderr"]),
        );
}
