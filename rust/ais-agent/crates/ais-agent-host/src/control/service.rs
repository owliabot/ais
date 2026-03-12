use std::{future::Future, pin::Pin};

use crate::{control::HostCommandOutcome, session::HostedRunCommand};

pub trait HostCommandService {
    fn handle(
        &mut self,
        command: HostedRunCommand,
    ) -> Pin<Box<dyn Future<Output = HostCommandOutcome> + Send + '_>>;
}
