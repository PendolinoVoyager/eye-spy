//! SCP - Simple Chat Protocol
//! A protocol using mDNS and simple messeges to negotiate stream sessions

use std::fmt::Display;

const SCP_HEADER: &[u8] = b"12345654321\n";
const SCP_END: &[u8] = b"1234564321\n";

/// Byte structure: <HEADER><COMMAND(16bits)><BODY><NEWLINE><END>
#[derive(Clone, Debug)]
pub struct ScpMessage {
    pub body: Vec<u8>,
    pub command: ScpCommand,
}

impl ScpMessage {
    /// #Panics
    /// Panics if the message cannot be constructed due to missing body when needed
    pub fn new(command: ScpCommand, body: &[u8]) -> Self {
        if command.requires_body() && body.is_empty() {
            panic!(
                "Tried to create an invalid SCP message: {:?}, {:?}",
                command, body
            );
        }
        ScpMessage {
            command,
            body: body.to_vec(),
        }
    }
    pub fn as_bytes(&self) -> Vec<u8> {
        [
            SCP_HEADER,
            &(self.command as u16).to_le_bytes(),
            &self.body,
            b"\n",
            SCP_END,
        ]
        .iter()
        .cloned()
        .flatten()
        .cloned()
        .collect()
    }
    pub fn deserialize(raw: &[u8]) -> Result<ScpMessage, SCPParseError> {
        const H_LEN: usize = SCP_HEADER.len();
        const C_LEN: usize = std::mem::size_of::<ScpCommand>();
        if !raw.starts_with(SCP_HEADER) {
            return Err(SCPParseError::BadStructure);
        }
        if !raw.ends_with(SCP_END) {
            return Err(SCPParseError::MissingEnd);
        }

        let (command_raw, raw) = raw[H_LEN..]
            .split_first_chunk::<C_LEN>()
            .ok_or(SCPParseError::MissingCommand)?;

        // Shouldn't panic: already checked for SCP_END
        let command;
        unsafe {
            command = std::mem::transmute::<[u8; C_LEN], ScpCommand>(*command_raw);
        }
        let (body_raw, end) = raw.split_at(raw.len() - H_LEN);
        // End must contains newline and SCP_END
        if &end[1..] != SCP_END {
            return Err(SCPParseError::BadStructure);
        }
        let body = body_raw.to_vec();
        if command.requires_body() && body.is_empty() {
            return Err(SCPParseError::MissingBody);
        }
        Ok(Self { command, body })
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
#[repr(u16)]
pub enum ScpCommand {
    // Start expects listener port
    Start,
    // Connection is established with an encryption key prepared earlier, skips key_share to later stages
    OwnKeyRequired,

    ReqGenerateKey,
    AckGenerateKey,
    KeyShare,
    /// Share the Preferences
    PreferencesShare,
    Ready,
    SimpleMessage,

    End,
}

impl ScpCommand {
    pub fn requires_body(&self) -> bool {
        match self {
            ScpCommand::Start => true,
            ScpCommand::OwnKeyRequired => false,
            ScpCommand::ReqGenerateKey => false,
            ScpCommand::AckGenerateKey => false,
            ScpCommand::KeyShare => true,
            ScpCommand::SimpleMessage => true,
            ScpCommand::PreferencesShare => true,
            ScpCommand::Ready => false,
            ScpCommand::End => false,
        }
    }
}

#[allow(unused)]
#[derive(Debug, PartialEq)]
pub enum SCPParseError {
    BadStructure,
    MissingBody,
    MissingCommand,
    MissingEnd,
}
impl Display for SCPParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SCPParseError::BadStructure => f.write_str(&format!(
                "Bad structure: SCP message should start with {} and end with {}",
                String::from_utf8_lossy(SCP_HEADER), String::from_utf8_lossy(SCP_END)
            )),
            SCPParseError::MissingBody => {
                f.write_str("Missing body: Some SCP messages expect body, but found empty")
            }
            SCPParseError::MissingEnd => f.write_str(&format!(
                "No ending: SCP message should end with {}",
                String::from_utf8_lossy(SCP_END)
            )),
            SCPParseError::MissingCommand => f.write_str("Missing command: SCP message requires a command (1 byte + newline) after the header.")
        }
    }
}

impl std::error::Error for SCPParseError {}

#[cfg(test)]
mod tests_scp {

    use crate::scp::{SCPParseError, ScpMessage};

    use super::{ScpCommand, SCP_END, SCP_HEADER};

    fn get_correct_message() -> Vec<u8> {
        [
            SCP_HEADER,
            &(ScpCommand::SimpleMessage as u16).to_le_bytes(),
            b"Hello\n",
            SCP_END,
        ]
        .iter()
        .cloned()
        .flatten()
        .cloned()
        .collect()
    }
    fn get_bad_message() -> Vec<u8> {
        [
            SCP_HEADER,
            &(ScpCommand::KeyShare as u16).to_le_bytes(),
            b"\n",
            SCP_END,
        ]
        .iter()
        .cloned()
        .flatten()
        .cloned()
        .collect()
    }
    #[test]
    fn test_scp_deserialization() {
        let msg = ScpMessage::deserialize(&get_correct_message());
        assert!(msg.is_ok());
        let msg = msg.unwrap();
        let string_msg = String::from_utf8_lossy(&msg.body);
        assert!(string_msg == "Hello")
    }
    #[test]
    fn test_bad_scp() {
        let msg = ScpMessage::deserialize(&get_bad_message());
        assert!(msg.is_err());
        assert!(msg.is_err_and(|e| e == SCPParseError::MissingBody))
    }
}
