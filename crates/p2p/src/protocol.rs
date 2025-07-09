use libp2p::StreamProtocol;
use std::{collections::HashSet, hash::Hash};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum Protocol {
    // validator -> worker
    ValidatorAuthentication,
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
            Protocol::ValidatorAuthentication => {
                StreamProtocol::new("/prime/validator_authentication/1.0.0")
            }
            Protocol::HardwareChallenge => StreamProtocol::new("/prime/hardware_challenge/1.0.0"),
            Protocol::Invite => StreamProtocol::new("/prime/invite/1.0.0"),
            Protocol::GetTaskLogs => StreamProtocol::new("/prime/get_task_logs/1.0.0"),
            Protocol::Restart => StreamProtocol::new("/prime/restart/1.0.0"),
            Protocol::General => StreamProtocol::new("/prime/general/1.0.0"),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Protocols(HashSet<Protocol>);

impl Protocols {
    pub(crate) fn new() -> Self {
        Self(HashSet::new())
    }

    pub(crate) fn with_validator_authentication(mut self) -> Self {
        self.0.insert(Protocol::ValidatorAuthentication);
        self
    }

    pub(crate) fn with_hardware_challenge(mut self) -> Self {
        self.0.insert(Protocol::HardwareChallenge);
        self
    }

    pub(crate) fn with_invite(mut self) -> Self {
        self.0.insert(Protocol::Invite);
        self
    }

    pub(crate) fn with_get_task_logs(mut self) -> Self {
        self.0.insert(Protocol::GetTaskLogs);
        self
    }

    pub(crate) fn with_restart(mut self) -> Self {
        self.0.insert(Protocol::Restart);
        self
    }

    pub(crate) fn with_general(mut self) -> Self {
        self.0.insert(Protocol::General);
        self
    }
}

impl IntoIterator for Protocols {
    type Item = Protocol;
    type IntoIter = std::collections::hash_set::IntoIter<Protocol>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}
