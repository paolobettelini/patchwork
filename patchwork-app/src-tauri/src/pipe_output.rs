/// Pipe output has no terminal driver to translate LF into CRLF.
/// Use only for pipe-backed composition output, never for PTY bytes.
#[derive(Default)]
pub(crate) struct PipeOutput {
    previous_was_cr: bool,
}

impl PipeOutput {
    pub(crate) fn for_terminal(&mut self, bytes: &[u8]) -> Vec<u8> {
        let mut output = Vec::with_capacity(bytes.len());
        for &byte in bytes {
            if byte == b'\n' && !self.previous_was_cr {
                output.push(b'\r');
            }
            output.push(byte);
            self.previous_was_cr = byte == b'\r';
        }
        output
    }
}
