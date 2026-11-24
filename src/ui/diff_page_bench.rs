use std::fmt::Write;
use std::time::{Duration, Instant};

use super::{DiffPage, DiffSession, theme};

#[test]
#[ignore = "release-mode benchmark; requires a display server"]
fn gtk_accept_benchmark() {
    gtk::init().unwrap();
    adw::init().unwrap();
    let unused = std::env::temp_dir().join(format!("notediff-bench-{}", std::process::id()));
    theme::install_at(&unused, None, &unused);
    let context = "This common text fills out the line to approximate a prose document. ";
    for (name, stride) in [("sparse", 4_000), ("dense", 100)] {
        let (mut baseline, mut live) = (String::new(), String::new());
        for line in 0..40_000 {
            let suffix = if line % stride == stride / 2 {
                "draft"
            } else {
                "saved"
            };
            writeln!(baseline, "{line:05} {context}saved").unwrap();
            writeln!(live, "{line:05} {context}{suffix}").unwrap();
        }
        let bytes = live.len();
        let page = DiffPage::new();
        let mut session = DiffSession::new(baseline, live);
        session.focus_relative(1);
        for _ in 0..session.diff().hunks.len() / 2 {
            session.focus_relative(1);
        }
        page.begin_review();
        page.render(&session);
        let median = |values: &mut [Duration]| {
            values.sort_unstable();
            values[values.len() / 2]
        };
        let mut accept = [[Duration::ZERO; 9]; 2];
        let mut undo = [[Duration::ZERO; 9]; 2];
        for i in 0..9 {
            let start = Instant::now();
            assert_eq!(session.accept_focused(), Ok(true));
            accept[0][i] = start.elapsed();
            let start = Instant::now();
            page.render(&session);
            accept[1][i] = start.elapsed();
            let start = Instant::now();
            assert!(session.undo());
            undo[0][i] = start.elapsed();
            let start = Instant::now();
            page.render(&session);
            undo[1][i] = start.elapsed();
        }
        eprintln!(
            "{name} ({bytes} bytes, {} hunks), median core + GTK buffer update: \
             accept {:?} + {:?}; undo {:?} + {:?}",
            session.diff().hunks.len(),
            median(&mut accept[0]),
            median(&mut accept[1]),
            median(&mut undo[0]),
            median(&mut undo[1]),
        );
    }
}
