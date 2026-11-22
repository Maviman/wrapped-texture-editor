p = r"E:\claude_programs\wrapped_texture_editor\target\round4\work\merged\crates\wte_app\src\app.rs"
s = open(p, encoding="utf-8", newline="").read()
assert "\r\n" not in s
def rep(old, new, count=1):
    global s
    assert s.count(old) == count, (old, s.count(old))
    s = s.replace(old, new)
rep("""        cpu_total: Duration,
        cpu_max: Duration,
    }
""", """        cpu_total: Duration,
        cpu_max: Duration,
        /// Round 4 (gate): `WTE_FRAME_LOG` was set at start: one `frame timing:` line per frame (the measurements of
        /// FEATURE_BRUSH P11 / P13 and FEATURE_TEXT 8.4 need single frames, not the 2-second windows).
        per_frame: bool,
        /// Interval from the previous frame's start to this frame's start (zero for the first frame).
        last_interval: Duration,
    }
""")
rep("""                    cpu_total: Duration::ZERO,
                    cpu_max: Duration::ZERO,
                });""", """                    cpu_total: Duration::ZERO,
                    cpu_max: Duration::ZERO,
                    per_frame: std::env::var_os("WTE_FRAME_LOG").is_some(),
                    last_interval: Duration::ZERO,
                });""")
rep("""                p.interval_total += d;
                p.interval_max = p.interval_max.max(d);
            }""", """                p.interval_total += d;
                p.interval_max = p.interval_max.max(d);
                p.last_interval = d;
            }""")
rep("""            p.cpu_max = p.cpu_max.max(cpu);
            for s in &mut p.segments {""", """            p.cpu_max = p.cpu_max.max(cpu);
            if p.per_frame {
                log::info!("{}", frame_line(p.last_interval, cpu, &p.segments));
            }
            for s in &mut p.segments {""")
rep("""    fn summary(p: &Profile, now: Instant) -> String {""", """    /// The per-frame line of `WTE_FRAME_LOG`: this frame's interval, its UI CPU time and every segment that ran in it.
    fn frame_line(interval: Duration, cpu: Duration, segments: &[Segment]) -> String {
        let parts: Vec<String> =
            segments.iter().filter(|s| s.frame > Duration::ZERO).map(|s| format!("{} {:.2}", s.name, ms(s.frame))).collect();
        format!("frame timing: interval {:.2} ms, ui cpu {:.2} ms; segments ms: {}", ms(interval), ms(cpu), parts.join(", "))
    }

    fn summary(p: &Profile, now: Instant) -> String {""")
rep("""                    assert!(summary(p, Instant::now()).contains("frame profile: 1 frames"));
                });
            })
            .join()
            .unwrap();
        }
""", """                    assert!(summary(p, Instant::now()).contains("frame profile: 1 frames"));
                });
            })
            .join()
            .unwrap();
        }

        /// Gate stage 3: the per-frame line of `WTE_FRAME_LOG` names the interval, the CPU time and only the segments
        /// that ran in the frame.
        #[test]
        fn the_per_frame_line_lists_the_segments_of_one_frame() {
            let segments = [
                Segment { name: "side panel", frame: Duration::from_micros(1250), ..Default::default() },
                Segment { name: "dispatch", ..Default::default() },
                Segment { name: "central", frame: Duration::from_millis(3), ..Default::default() },
            ];
            let line = frame_line(Duration::from_micros(16_700), Duration::from_micros(4_500), &segments);
            assert_eq!(line, "frame timing: interval 16.70 ms, ui cpu 4.50 ms; segments ms: side panel 1.25, central 3.00");
        }
""")
open(p, "w", encoding="utf-8", newline="").write(s)
print("ok")
