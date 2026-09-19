use std::io::Write;

pub(crate) fn format_record(
    buf: &mut env_logger::fmt::Formatter,
    record: &log::Record<'_>,
) -> std::io::Result<()> {
    let text =
        crate::services::tools::redact_secrets(&format!("{} {}", record.target(), record.args()));
    writeln!(buf, "[{} {}] {}", buf.timestamp(), record.level(), text)
}
