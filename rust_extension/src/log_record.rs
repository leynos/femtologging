#[allow(dead_code)]
#[derive(Debug)]
pub struct FemtoLogRecord<'a> {
    pub level: &'a str,
    pub message: &'a str,
}
