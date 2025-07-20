use tokio::{
    io,
    signal::unix::{signal, SignalKind},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

// Spawn a task to listen for signals and call `cancel` on the cancellation token
// when a signal is received.
//
// Returns a handle to the spawned task.
pub fn trigger_cancellation_on_signal(
    cancellation_token: CancellationToken,
) -> io::Result<JoinHandle<()>> {
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sighup = signal(SignalKind::hangup())?;
    let mut sigquit = signal(SignalKind::quit())?;

    let signal_handle = tokio::spawn(async move {
        tokio::select! {
            _ = sigterm.recv() => {
                log::info!("Received termination signal");
            }
            _ = sigint.recv() => {
                log::info!("Received interrupt signal");
            }
            _ = sighup.recv() => {
                log::info!("Received hangup signal");
            }
            _ = sigquit.recv() => {
                log::info!("Received quit signal");
            }
        }
        cancellation_token.cancel();
    });

    Ok(signal_handle)
}
