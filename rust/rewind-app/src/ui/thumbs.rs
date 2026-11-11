//! Cached clip thumbnails, shared by the Clips grid and Home's recent row.
//!
//! Missing thumbnails are generated on one worker thread — a serial FFmpeg spawn is
//! plenty for a library that is mostly already done — and decoding is budgeted per
//! frame so opening a page on a large library does not freeze it.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::SystemTime;

use eframe::egui;

use crate::controller::ClipEntry;
use crate::ui::load_texture;

/// Thumbnail JPEGs decoded per frame.
const DECODE_BUDGET: usize = 4;

struct Cached {
    texture: egui::TextureHandle,
    stamp: Option<SystemTime>,
}

pub struct Thumbs {
    textures: HashMap<PathBuf, Cached>,
    failed: HashSet<PathBuf>,
    jobs: Sender<PathBuf>,
    done: Receiver<(PathBuf, Option<PathBuf>)>,
    inflight: HashSet<PathBuf>,
    budget: usize,
}

impl Thumbs {
    pub fn new(ctx: &egui::Context) -> Thumbs {
        let (jobs, job_rx) = channel::<PathBuf>();
        let (done, done_rx) = channel();
        let wake = ctx.clone();
        std::thread::Builder::new()
            .name("rewind-thumbs".into())
            .spawn(move || {
                let ffmpeg = rewind_engine::find_ffmpeg();
                for video in job_rx {
                    let thumbnail = ffmpeg.as_ref().and_then(|ffmpeg| {
                        // Probe and record the duration here, as the Python worker did:
                        // it is the only place a clip's length is known before the user
                        // opens it, and both pages show it.
                        let key = rewind_engine::paths::meta_key(&video);
                        let duration = rewind_engine::meta::meta_duration(&key).or_else(|| {
                            let probed = rewind_engine::ffmpeg::probe_duration(ffmpeg, &video)?;
                            let _ = rewind_engine::meta::update_meta(
                                &key,
                                &[(
                                    "duration",
                                    serde_json::json!((probed * 100.0).round() / 100.0),
                                )],
                            );
                            Some(probed)
                        });
                        rewind_engine::meta::make_thumbnail(ffmpeg, &video, duration)
                    });
                    let _ = done.send((video, thumbnail));
                    wake.request_repaint();
                }
            })
            .ok();
        Thumbs {
            textures: HashMap::new(),
            failed: HashSet::new(),
            jobs,
            done: done_rx,
            inflight: HashSet::new(),
            budget: DECODE_BUDGET,
        }
    }

    /// Start a frame: reset the decode budget and take in finished jobs.
    ///
    /// Returns whether a thumbnail became available, so the page can re-list and pick up
    /// the duration the worker just recorded.
    pub fn begin(&mut self) -> bool {
        self.budget = DECODE_BUDGET;
        let mut produced = false;
        for (video, thumbnail) in self.done.try_iter() {
            self.inflight.remove(&video);
            match thumbnail {
                Some(_) => produced = true,
                None => {
                    self.failed.insert(video);
                }
            }
        }
        produced
    }

    /// Ask the worker for a thumbnail (and, for videos, a duration) — once per clip.
    pub fn request(&mut self, video: PathBuf) {
        if self.failed.contains(&video) {
            return;
        }
        if self.inflight.insert(video.clone()) {
            let _ = self.jobs.send(video);
        }
    }

    /// The cached texture for a clip, decoding it when it is new or has changed.
    pub fn texture(
        &mut self,
        ctx: &egui::Context,
        entry: &ClipEntry,
    ) -> Option<egui::TextureHandle> {
        let path = entry.thumbnail.as_ref()?;
        let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if let Some(cached) = self.textures.get(path) {
            if cached.stamp == stamp {
                return Some(cached.texture.clone());
            }
        }
        if self.budget == 0 {
            return self.textures.get(path).map(|cached| cached.texture.clone());
        }
        self.budget -= 1;
        let texture = load_texture(ctx, &format!("thumb:{}", path.display()), path)?;
        self.textures.insert(
            path.clone(),
            Cached {
                texture: texture.clone(),
                stamp,
            },
        );
        Some(texture)
    }
}
