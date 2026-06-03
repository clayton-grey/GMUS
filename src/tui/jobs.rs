use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use anyhow::{anyhow, Result};

use crate::config::AppPaths;
use crate::{db, library};

pub(super) struct LibraryJobRunner {
    command: String,
    receiver: Receiver<Result<library::LibraryJobResult>>,
}

impl LibraryJobRunner {
    pub(super) fn spawn(command: String, paths: AppPaths, job: library::LibraryJob) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result =
                db::open(&paths.db_path).and_then(|conn| library::run_job(&conn, &paths, job));
            let _ = sender.send(result);
        });

        Self { command, receiver }
    }

    pub(super) fn command(&self) -> &str {
        &self.command
    }

    pub(super) fn try_finish(&self) -> Result<Option<Result<library::LibraryJobResult>>> {
        match self.receiver.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(anyhow!(
                "library job worker disconnected before reporting a result"
            )),
        }
    }
}
