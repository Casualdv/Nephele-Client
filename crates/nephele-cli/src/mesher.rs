//! Background chunk meshing.
//!
//! Building a chunk mesh is pure CPU work over a block snapshot, so it does not
//! need to happen on the render thread. A small pool of workers turns column
//! snapshots into vertex data; the main thread only uploads the results.
//!
//! Keeping meshing off the main thread is what stops the frame from overrunning
//! its vsync slot when a burst of chunks arrives (at login, or when sprinting
//! into new terrain).

use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use crate::perf;
use nephele_assets::TextureArray;
use nephele_render::{mesh_column, ColumnMeshJob, MeshData};

pub struct MeshPool {
    jobs: Sender<ColumnMeshJob>,
    results: Receiver<((i32, i32), MeshData)>,
    /// Keys handed to a worker and not yet collected.
    in_flight: HashSet<(i32, i32)>,
    capacity: usize,
    workers: usize,
}

impl MeshPool {
    /// Spawn one worker per physical core, less one for the render thread.
    pub fn new(layers: Arc<TextureArray>) -> Self {
        let cores = thread::available_parallelism()
            .map(|cores| cores.get())
            .unwrap_or(4);
        let workers = cores.saturating_sub(1).clamp(2, 8);
        Self::with_workers(layers, workers)
    }

    pub fn with_workers(layers: Arc<TextureArray>, workers: usize) -> Self {
        let (job_tx, job_rx) = channel::<ColumnMeshJob>();
        let (result_tx, result_rx) = channel::<((i32, i32), MeshData)>();
        let job_rx = Arc::new(Mutex::new(job_rx));

        for index in 0..workers {
            let job_rx = Arc::clone(&job_rx);
            let result_tx = result_tx.clone();
            let layers = Arc::clone(&layers);
            thread::Builder::new()
                .name(format!("mesh-{index}"))
                .spawn(move || loop {
                    // The lock is only held while waiting for a job, so workers
                    // still mesh concurrently.
                    let received = {
                        let rx = job_rx.lock().unwrap_or_else(|err| err.into_inner());
                        rx.recv()
                    };
                    let Ok(job) = received else {
                        // The pool was dropped.
                        break;
                    };
                    let key = job.key;
                    let mesh = if perf::enabled() {
                        let started_at = Instant::now();
                        let mesh = mesh_column(&job, &layers);
                        perf::record_mesh_build_time(started_at.elapsed());
                        mesh
                    } else {
                        mesh_column(&job, &layers)
                    };
                    if result_tx.send((key, mesh)).is_err() {
                        break;
                    }
                })
                .expect("spawn mesh worker");
        }

        Self {
            jobs: job_tx,
            results: result_rx,
            in_flight: HashSet::new(),
            capacity: workers * 6,
            workers,
        }
    }

    pub fn workers(&self) -> usize {
        self.workers
    }

    /// Most jobs that may be queued at once, so a huge burst can't swamp memory.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn in_flight(&self) -> usize {
        self.in_flight.len()
    }

    pub fn is_in_flight(&self, key: (i32, i32)) -> bool {
        self.in_flight.contains(&key)
    }

    pub fn has_capacity(&self) -> bool {
        self.in_flight.len() < self.capacity
    }

    /// Hand a snapshot to a worker. Returns `false` if the pool is saturated.
    pub fn submit(&mut self, job: ColumnMeshJob) -> bool {
        if !self.has_capacity() {
            return false;
        }
        let key = job.key;
        self.in_flight.insert(key);
        if self.jobs.send(job).is_err() {
            self.in_flight.remove(&key);
            return false;
        }
        true
    }

    /// Pop one finished mesh, if any.
    pub fn try_recv(&mut self) -> Option<((i32, i32), MeshData)> {
        match self.results.try_recv() {
            Ok((key, mesh)) => {
                self.in_flight.remove(&key);
                Some((key, mesh))
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_return_meshes_for_every_submitted_job() {
        let layers = Arc::new(TextureArray::from_tiles(vec![(
            "stone",
            image::RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255])),
        )]));
        let mut pool = MeshPool::with_workers(layers, 2);

        let mut world = nephele_world::World::new(0);
        let mut column = nephele_world::ChunkColumn::empty(0, 0);
        column.set_block(1, 40, 1, nephele_world::BlockState::STONE);
        world.insert(column);

        assert!(pool.submit(ColumnMeshJob::from_world((0, 0), &world)));
        assert_eq!(pool.in_flight(), 1);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut got = None;
        while std::time::Instant::now() < deadline {
            if let Some(result) = pool.try_recv() {
                got = Some(result);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let (key, mesh) = got.expect("worker returned a mesh");
        assert_eq!(key, (0, 0));
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(pool.in_flight(), 0);
    }
}
