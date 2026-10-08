//! Structural man-page checks (acceptance: `flowshot.1` +
//! `flowshot-config.5` exist and carry valid roff structure; the actual
//! `man --warnings -l` render is live QA, recorded in the evidence file).

use clap::CommandFactory;
use flowshot_cli::args::Cli;
use flowshot_cli::config_man::FLOWSHOT_CONFIG_ROFF;

#[test]
fn flowshot_1_renders_from_the_clap_surface() {
    let mut roff = Vec::new();
    clap_mangen::Man::new(Cli::command())
        .render(&mut roff)
        .unwrap_or_else(|error| panic!("{error}"));
    let text = String::from_utf8(roff).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        text.contains(".TH flowshot 1"),
        "a man page must carry the .TH title line"
    );
    for token in ["flowshot", "capture", "completions", "daemon"] {
        assert!(text.contains(token), "flowshot.1 misses {token}");
    }
}

#[test]
fn the_generated_page_is_named_flowshot_1() {
    let page = clap_mangen::Man::new(Cli::command());
    assert_eq!(page.get_filename(), "flowshot.1");
}

#[test]
fn flowshot_config_5_is_a_section_5_file_formats_page() {
    assert!(FLOWSHOT_CONFIG_ROFF.starts_with(".TH FLOWSHOT-CONFIG 5"));
    for token in [
        "flowshot.toml",
        "config_version",
        "[capture]",
        "[save]",
        "[editor]",
        "[tools.",
        "[pin]",
        "[upload]",
        "[ui]",
        "[daemon]",
        "client_id",
        "actions",
        "SEE ALSO",
    ] {
        assert!(
            FLOWSHOT_CONFIG_ROFF.contains(token),
            "flowshot-config.5 misses {token}"
        );
    }
}
