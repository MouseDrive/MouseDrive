#![deny(unsafe_code)]

use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{self, Receiver, SendError, Sender, TryRecvError};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

const THREAD_NAME: &str = "disk";

type Job = Box<dyn FnOnce() + Send>;

pub struct DiskWriter {
    queue: Mutex<Option<Sender<Job>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum JobState<T> {
    Running,
    Done(T),
    Lost,
}

#[derive(Debug)]
pub struct Pending<T> {
    result: Receiver<T>,
}

impl<T> Pending<T> {
    pub fn state(&self) -> JobState<T> {
        match self.result.try_recv() {
            Ok(value) => JobState::Done(value),
            Err(TryRecvError::Empty) => JobState::Running,
            Err(TryRecvError::Disconnected) => JobState::Lost,
        }
    }

    pub fn wait(self) -> Option<T> {
        self.result.recv().ok()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn run_job(job: Job) {
    if std::panic::catch_unwind(AssertUnwindSafe(job)).is_err() {
        crate::log::error("bir disk işi panikle bitti; sıradaki işler sürüyor");
    }
}

fn run_jobs(jobs: &Receiver<Job>) {
    jobs.iter().for_each(run_job);
}

impl DiskWriter {
    pub fn start() -> Self {
        let (queue, jobs) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name(THREAD_NAME.into())
            .spawn(move || run_jobs(&jobs));
        match spawned {
            Ok(thread) => Self {
                queue: Mutex::new(Some(queue)),
                thread: Mutex::new(Some(thread)),
            },
            Err(e) => {
                crate::log::error(&format!(
                    "disk thread'i başlatılamadı: {e}; dosyalar pencere thread'inde yazılacak"
                ));
                Self {
                    queue: Mutex::new(None),
                    thread: Mutex::new(None),
                }
            }
        }
    }

    pub fn submit<T, F>(&self, job: F) -> Pending<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let (reply, result) = mpsc::sync_channel(1);
        let job: Job = Box::new(move || {
            if reply.send(job()).is_err() {
                crate::log::info("disk işinin sonucunu bekleyen kalmadı");
            }
        });
        if let Err(SendError(job)) = self.send(job) {
            run_job(job);
        }
        Pending { result }
    }

    pub fn run<T, F>(&self, job: F) -> Option<T>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        self.submit(job).wait()
    }

    pub fn flush(&self) {
        if self.run(|| ()).is_none() {
            crate::log::warn("disk kuyruğu boşaltılırken bir iş yarıda kaldı");
        }
    }

    pub fn finish(&self) {
        drop(lock(&self.queue).take());
        let Some(thread) = lock(&self.thread).take() else {
            return;
        };
        if thread.join().is_err() {
            crate::log::error("disk thread'i panikle bitti");
        }
    }

    fn send(&self, job: Job) -> Result<(), SendError<Job>> {
        match lock(&self.queue).as_ref() {
            Some(queue) => queue.send(job),
            None => Err(SendError(job)),
        }
    }
}

impl Drop for DiskWriter {
    fn drop(&mut self) {
        self.finish();
    }
}
