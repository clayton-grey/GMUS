use std::thread::{self, JoinHandle};

use anyhow::{anyhow, Result};

use crate::config::AppPaths;
use crate::{db, library};

pub(super) struct LibraryJobRunner {
    command: String,
    worker: Option<JoinHandle<Result<library::LibraryJobResult>>>,
}

impl LibraryJobRunner {
    pub(super) fn spawn(command: String, paths: AppPaths, job: library::LibraryJob) -> Self {
        let worker = thread::spawn(move || {
            db::open(&paths.db_path).and_then(|conn| library::run_job(&conn, &paths, job))
        });

        Self {
            command,
            worker: Some(worker),
        }
    }

    pub(super) fn command(&self) -> &str {
        &self.command
    }

    pub(super) fn try_finish(&mut self) -> Result<Option<Result<library::LibraryJobResult>>> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            return Ok(None);
        }
        self.join_worker().map(Some)
    }

    pub(super) fn finish(&mut self) -> Result<library::LibraryJobResult> {
        self.join_worker()?
    }

    fn join_worker(&mut self) -> Result<Result<library::LibraryJobResult>> {
        let worker = self
            .worker
            .take()
            .ok_or_else(|| anyhow!("library job worker was already joined"))?;
        worker
            .join()
            .map_err(|_| anyhow!("library job worker panicked"))
    }
}

impl Drop for LibraryJobRunner {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    };
    use std::time::Duration;

    use super::*;

    fn runner(worker: JoinHandle<Result<library::LibraryJobResult>>) -> LibraryJobRunner {
        LibraryJobRunner {
            command: String::from(":library-update"),
            worker: Some(worker),
        }
    }

    #[test]
    fn finish_joins_worker_and_returns_result() {
        let mut runner = runner(thread::spawn(|| {
            Ok(library::LibraryJobResult::NoActiveRoots)
        }));

        let result = runner.finish().unwrap();

        assert!(matches!(result, library::LibraryJobResult::NoActiveRoots));
        assert!(runner.worker.is_none());
    }

    #[test]
    fn polling_remains_nonblocking_until_worker_finishes() {
        let (release, wait) = mpsc::channel();
        let mut runner = runner(thread::spawn(move || {
            wait.recv().unwrap();
            Ok(library::LibraryJobResult::NoActiveRoots)
        }));

        assert!(runner.try_finish().unwrap().is_none());

        release.send(()).unwrap();
        while runner
            .worker
            .as_ref()
            .is_some_and(|worker| !worker.is_finished())
        {
            thread::yield_now();
        }
        assert!(matches!(
            runner.try_finish().unwrap().unwrap().unwrap(),
            library::LibraryJobResult::NoActiveRoots
        ));
        assert!(runner.worker.is_none());
    }

    #[test]
    fn worker_panic_is_reported() {
        let mut runner = runner(thread::spawn(|| panic!("worker failed")));

        let error = runner.finish().unwrap_err();

        assert_eq!(error.to_string(), "library job worker panicked");
    }

    #[test]
    fn dropping_runner_waits_for_worker() {
        let started = Arc::new(Barrier::new(2));
        let finished = Arc::new(AtomicBool::new(false));
        let worker_started = Arc::clone(&started);
        let worker_finished = Arc::clone(&finished);
        let runner = runner(thread::spawn(move || {
            worker_started.wait();
            thread::sleep(Duration::from_millis(10));
            worker_finished.store(true, Ordering::SeqCst);
            Ok(library::LibraryJobResult::NoActiveRoots)
        }));
        started.wait();

        drop(runner);

        assert!(finished.load(Ordering::SeqCst));
    }
}
