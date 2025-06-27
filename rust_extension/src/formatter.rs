pub trait FemtoFormatter: Send + Sync {
    fn format(&self, record: &crate::log_record::FemtoLogRecord) -> String;
}

pub struct DefaultFormatter;

impl FemtoFormatter for DefaultFormatter {
    fn format(&self, record: &crate::log_record::FemtoLogRecord) -> String {
        format!("{}: {} - {}", record.logger, record.level, record.message)
    }
}
