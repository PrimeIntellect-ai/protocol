use libp2p::StreamProtocol;
use std::{collections::HashSet, hash::Hash};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Protocol {
    // validator or orchestrator -> worker
    Authentication,
    // validator -> worker
    HardwareChallenge,
    // orchestrator -> worker
    Invite,
    // any -> worker
    GetTaskLogs,
    // any -> worker
    Restart,
    // any -> any
    General,
}

impl Protocol {
    pub(crate) fn as_stream_protocol(&self) -> StreamProtocol {
        match self {
            Protocol::Authentication => StreamProtocol::new("/prime/authentication/1.0.0"),
            Protocol::HardwareChallenge => StreamProtocol::new("/prime/hardware_challenge/1.0.0"),
            Protocol::Invite => StreamProtocol::new("/prime/invite/1.0.0"),
            Protocol::GetTaskLogs => StreamProtocol::new("/prime/get_task_logs/1.0.0"),
            Protocol::Restart => StreamProtocol::new("/prime/restart/1.0.0"),
            Protocol::General => StreamProtocol::new("/prime/general/1.0.0"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Protocols(HashSet<Protocol>);

impl Default for Protocols {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocols {
    pub fn new() -> Self {
        Self(HashSet::new())
    }

    pub fn has_authentication(&self) -> bool {
        self.0.contains(&Protocol::Authentication)
    }

    pub fn has_hardware_challenge(&self) -> bool {
        self.0.contains(&Protocol::HardwareChallenge)
    }

    pub fn has_invite(&self) -> bool {
        self.0.contains(&Protocol::Invite)
    }

    pub fn has_get_task_logs(&self) -> bool {
        self.0.contains(&Protocol::GetTaskLogs)
    }

    pub fn has_restart(&self) -> bool {
        self.0.contains(&Protocol::Restart)
    }

    pub fn has_general(&self) -> bool {
        self.0.contains(&Protocol::General)
    }

    pub fn with_authentication(mut self) -> Self {
        self.0.insert(Protocol::Authentication);
        self
    }

    pub fn with_hardware_challenge(mut self) -> Self {
        self.0.insert(Protocol::HardwareChallenge);
        self
    }

    pub fn with_invite(mut self) -> Self {
        self.0.insert(Protocol::Invite);
        self
    }

    pub fn with_get_task_logs(mut self) -> Self {
        self.0.insert(Protocol::GetTaskLogs);
        self
    }

    pub fn with_restart(mut self) -> Self {
        self.0.insert(Protocol::Restart);
        self
    }

    pub fn with_general(mut self) -> Self {
        self.0.insert(Protocol::General);
        self
    }

    pub(crate) fn join(&mut self, other: Protocols) {
        self.0.extend(other.0);
    }
}

impl IntoIterator for Protocols {
    type Item = Protocol;
    type IntoIter = std::collections::hash_set::IntoIter<Protocol>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
