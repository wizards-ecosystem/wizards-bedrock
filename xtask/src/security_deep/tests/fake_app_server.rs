use super::*;

use std::io::{BufRead, Write};

pub(super) fn read_value(
    reader: &mut impl BufRead,
    client: &str,
    label: &str,
) -> Result<JsonValue> {
    let mut line = Vec::new();
    ensure!(
        reader.read_until(b'\n', &mut line)? != 0,
        "{client} closed before {label}"
    );
    ensure!(
        line.last() == Some(&b'\n')
            && line.iter().filter(|byte| **byte == b'\n').count() == 1
            && !line.contains(&b'\r'),
        "{client} used noncanonical framing for {label}"
    );
    line.pop();
    let value = parse_json(&line, label)?;
    ensure!(
        render_json(&value)?.as_bytes() == line,
        "{client} did not use canonical JSON for {label}"
    );
    Ok(value)
}

pub(super) fn expect_value(
    reader: &mut impl BufRead,
    expected: JsonValue,
    client: &str,
    label: &str,
) -> Result<()> {
    ensure!(
        read_value(reader, client, label)? == expected,
        "{client} {label} differs"
    );
    Ok(())
}

pub(super) fn send_value(writer: &mut impl Write, value: &JsonValue) -> Result<()> {
    writer.write_all(render_json(value)?.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

pub(super) fn wait_until_ready(
    dialogue: &mut LineDialogue,
    ready: &[u8],
    server: &str,
) -> Result<()> {
    let mut observed = 0usize;
    for _ in 0..8 {
        let line = dialogue
            .receive_line()?
            .with_context(|| format!("{server} closed before its ready line"))?;
        observed = observed
            .checked_add(line.len())
            .with_context(|| format!("{server} ready prelude length overflowed"))?;
        ensure!(observed <= 4096, "{server} ready prelude is excessive");
        if line.ends_with(ready) {
            return Ok(());
        }
    }
    bail!("{server} omitted its ready line")
}
