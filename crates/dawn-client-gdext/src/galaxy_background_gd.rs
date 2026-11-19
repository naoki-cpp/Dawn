use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, TryRecvError},
    Arc,
};

use dawn_client_core::{generate_background, GalaxyBackground, GalaxyConfig};
use godot::{classes::image::Format, classes::Image, prelude::*};

/// Completed CPU data; Godot resources are created only on the calling main thread.
#[derive(Debug, GodotClass)]
#[class(no_init, base=RefCounted)]
pub struct GalaxyBackgroundResult {
    data: Arc<GalaxyBackground>,
}

#[godot_api]
impl GalaxyBackgroundResult {
    #[func]
    fn panorama(&self) -> Option<Gd<Image>> {
        let bytes: Vec<u8> = self
            .data
            .pixels
            .iter()
            .flatten()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        Image::create_from_data(
            self.data.width as i32,
            self.data.height as i32,
            false,
            Format::RGBF,
            &PackedByteArray::from(bytes.as_slice()),
        )
    }

    #[func]
    fn star_directions(&self) -> PackedVector3Array {
        self.data
            .stars
            .iter()
            .map(|star| {
                Vector3::new(
                    star.direction[0] as f32,
                    star.direction[1] as f32,
                    star.direction[2] as f32,
                )
            })
            .collect()
    }

    #[func]
    fn star_colors(&self) -> PackedColorArray {
        self.data
            .stars
            .iter()
            .map(|star| Color::from_rgb(star.color[0], star.color[1], star.color[2]))
            .collect()
    }

    #[func]
    fn star_fluxes(&self) -> PackedFloat64Array {
        self.data.stars.iter().map(|star| star.flux).collect()
    }
}

#[derive(Debug)]
struct RunningGeneration {
    config: GalaxyConfig,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<GalaxyBackground, String>>,
}

impl Drop for RunningGeneration {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// One worker and one desired request: replacing requests never queues unbounded work.
#[derive(Debug, Default)]
struct BackgroundGeneration {
    desired: Option<GalaxyConfig>,
    running: Option<RunningGeneration>,
    cache: Option<(GalaxyConfig, Arc<GalaxyBackground>)>,
    deliver: bool,
    error: Option<String>,
}

impl BackgroundGeneration {
    fn request(&mut self, config: GalaxyConfig) {
        if self.desired.as_ref() == Some(&config) {
            return;
        }
        if let Some(running) = &self.running {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.deliver = self.cache.as_ref().is_some_and(|(key, _)| key == &config);
        self.desired = Some(config);
        self.error = None;
    }

    fn poll(&mut self) -> Option<Arc<GalaxyBackground>> {
        if let Some(running) = &self.running {
            match running.receiver.try_recv() {
                Ok(result) => {
                    let finished = self.running.take().unwrap();
                    if self.desired.as_ref() == Some(&finished.config)
                        && !finished.cancel.load(Ordering::Relaxed)
                    {
                        match result {
                            Ok(data) => {
                                self.cache = Some((finished.config.clone(), Arc::new(data)));
                                self.deliver = true;
                            }
                            Err(error) => {
                                self.error = Some(error);
                                self.desire_failed();
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    let finished = self.running.take().unwrap();
                    if self.desired.as_ref() == Some(&finished.config)
                        && !finished.cancel.load(Ordering::Relaxed)
                    {
                        self.error = Some("Background generation worker stopped".into());
                        self.desire_failed();
                    }
                }
            }
        }
        if self.deliver {
            self.deliver = false;
            return self.cache.as_ref().map(|(_, data)| Arc::clone(data));
        }
        let desired = self.desired.as_ref()?;
        let cached = self.cache.as_ref().is_some_and(|(key, _)| key == desired);
        if self.running.is_none() && !cached {
            let config = desired.clone();
            let worker_config = config.clone();
            let cancel = Arc::new(AtomicBool::new(false));
            let worker_cancel = Arc::clone(&cancel);
            let (sender, receiver) = mpsc::sync_channel(1);
            match std::thread::Builder::new()
                .name("galaxy-background".into())
                .spawn(move || {
                    let result = generate_background(&worker_config, &worker_cancel)
                        .map_err(|error| error.to_string());
                    let _ = sender.send(result);
                }) {
                Ok(_) => {
                    self.running = Some(RunningGeneration {
                        config,
                        cancel,
                        receiver,
                    })
                }
                Err(error) => {
                    self.error = Some(format!("Cannot start background generation: {error}"));
                    self.desire_failed();
                }
            }
        }
        None
    }

    fn desire_failed(&mut self) {
        self.desired = None;
        self.deliver = false;
    }
}

/// Pollable adapter for cancellable CPU-only generation; no worker touches Godot.
#[derive(Debug, GodotClass)]
#[class(init, base=RefCounted)]
pub struct GalaxyBackgroundGenerator {
    generation: BackgroundGeneration,
}

#[godot_api]
impl GalaxyBackgroundGenerator {
    #[func]
    fn request(&mut self, seed: i64, width: i64, height: i64, star_count: i64) {
        let (Ok(width), Ok(height), Ok(star_count)) = (
            u32::try_from(width),
            u32::try_from(height),
            u32::try_from(star_count),
        ) else {
            self.cancel();
            self.generation.error =
                Some("Invalid negative or overflowing background dimensions".into());
            return;
        };
        self.generation.request(GalaxyConfig {
            seed: seed as u64,
            width,
            height,
            star_count,
            ..GalaxyConfig::default()
        });
    }

    #[func]
    fn poll(&mut self) -> Option<Gd<GalaxyBackgroundResult>> {
        self.generation
            .poll()
            .map(|data| Gd::from_object(GalaxyBackgroundResult { data }))
    }

    #[func]
    fn take_error(&mut self) -> GString {
        self.generation
            .error
            .take()
            .unwrap_or_default()
            .as_str()
            .into()
    }

    #[func]
    fn cancel(&mut self) {
        if let Some(running) = &self.generation.running {
            running.cancel.store(true, Ordering::Relaxed);
        }
        self.generation.desired = None;
        self.generation.deliver = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn config(seed: u64) -> GalaxyConfig {
        GalaxyConfig {
            seed,
            width: 16,
            height: 8,
            star_count: 16,
            ..GalaxyConfig::default()
        }
    }

    fn finish(job: &mut BackgroundGeneration) -> Arc<GalaxyBackground> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(data) = job.poll() {
                return data;
            }
            assert!(
                Instant::now() < deadline,
                "generation timeout: {:?}",
                job.error
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn same_settings_reuse_one_completed_result() {
        let mut job = BackgroundGeneration::default();
        job.request(config(1));
        let first = finish(&mut job);
        job.request(config(1));
        assert!(job.poll().is_none());
        job.request(config(2));
        job.request(config(1));
        assert!(Arc::ptr_eq(&first, &finish(&mut job)));
    }

    #[test]
    fn replaced_generation_cannot_overwrite_latest_request() {
        let mut job = BackgroundGeneration::default();
        job.request(config(1));
        assert!(job.poll().is_none());
        job.request(config(2));
        let actual = finish(&mut job);
        let expected = generate_background(&config(2), &AtomicBool::new(false)).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
        assert_eq!(job.cache.as_ref().unwrap().0.seed, 2);
        assert!(job.poll().is_none());
    }

    #[test]
    fn returning_to_a_cancelled_configuration_regenerates_it() {
        let mut job = BackgroundGeneration::default();
        job.request(config(1));
        job.poll();
        job.request(config(2));
        job.request(config(1));
        let actual = finish(&mut job);
        let expected = generate_background(&config(1), &AtomicBool::new(false)).unwrap();
        assert_eq!(actual.pixels, expected.pixels);
        assert!(job.error.is_none());
    }

    #[test]
    fn invalid_generation_fails_once_without_discarding_cache() {
        let mut job = BackgroundGeneration::default();
        job.request(config(1));
        let cached = finish(&mut job);
        let mut invalid = config(2);
        invalid.width = 0;
        job.request(invalid);
        let deadline = Instant::now() + Duration::from_secs(10);
        while job.error.is_none() {
            assert!(job.poll().is_none());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(job.desired.is_none());
        job.request(config(1));
        assert!(Arc::ptr_eq(&cached, &finish(&mut job)));
    }

    #[test]
    fn dropping_running_job_requests_cancellation() {
        let mut job = BackgroundGeneration::default();
        job.request(config(1));
        job.poll();
        let cancel = Arc::clone(&job.running.as_ref().unwrap().cancel);
        drop(job);
        assert!(cancel.load(Ordering::Relaxed));
    }
}
