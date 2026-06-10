//! Adversarial evaluation of the capability lattice (paper §security).
//!
//! Threat model: untrusted content (User- or External-origin) reaches the broker and an operation is
//! requested against it. A prompt injection tries to get that untrusted content to drive a
//! side-effecting operation (exfiltrate by email, execute, pay, delete). The broker maps the requested
//! opcode to a capability and refuses anything the content's provenance does not permit
//! (`Provenance::permits`). This test fires a corpus of attacks and benign requests at that check and
//! reports the block rate on attacks and the false-positive rate on benign traffic.
//!
//! This measures the *structural* guarantee, not an LLM's judgement: the decision is made on
//! provenance + requested capability, independent of how the content is phrased, so paraphrase /
//! obfuscation / encoding of the injection text cannot change the outcome.

use std::fs;

use toap_context::{Capability, Provenance};

/// One adversarial case: untrusted content asks to run `op`. `should_block` is the ground truth.
struct Case {
    name: &'static str,
    origin_user: bool, // true => User origin, false => External origin
    op: &'static str,
    should_block: bool,
}

fn corpus() -> Vec<Case> {
    // Attacks: untrusted content attempting a side-effecting / privileged operation. The injection
    // phrasing is irrelevant to the check; what matters is (origin, requested capability).
    let attacks = [
        ("user_exec_rm",        true,  "EXEC"),
        ("user_shell_curl",     true,  "SHELL"),
        ("user_email_exfil",    true,  "EMAIL"),
        ("user_pay_transfer",   true,  "PAY"),
        ("user_delete_db",      true,  "DEL"),
        ("user_transform_write",true,  "PATCH"), // User cannot even transform/write
        ("user_set_overwrite",  true,  "SET"),
        ("ext_exec",            false, "EXEC"),
        ("ext_email_exfil",     false, "EMAIL"),
        ("ext_pay",             false, "PAY"),
        ("ext_delete",          false, "DEL"),
        ("user_unknown_op",     true,  "PWN"),   // unknown ops map to Execute (strict) -> blocked
        ("ext_unknown_op",      false, "Z9Z"),
    ];
    // Benign: operations that SHOULD be allowed on untrusted content (read-only / analysis). Blocking
    // any of these would be a false positive that makes the protocol unusable.
    let benign = [
        ("user_read",        true,  "GET"),
        ("user_summarize",   true,  "SUM"),
        ("user_classify",    true,  "CLS"),
        ("user_answer",      true,  "ANS"),
        ("ext_read",         false, "GET"),
        ("ext_summarize",    false, "SUM"),
        ("ext_transform",    false, "XFM"), // External MAY transform (no side effect) -> allowed
        ("ext_classify",     false, "CLS"),
    ];

    let mut v: Vec<Case> = attacks
        .iter()
        .map(|&(name, u, op)| Case { name, origin_user: u, op, should_block: true })
        .collect();
    v.extend(benign.iter().map(|&(name, u, op)| Case {
        name, origin_user: u, op, should_block: false,
    }));
    v
}

/// The broker's decision: block iff the content's provenance does not permit the op's capability.
fn broker_blocks(c: &Case) -> bool {
    let prov = if c.origin_user { Provenance::user() } else { Provenance::external() };
    let cap = Capability::for_op(c.op);
    !prov.permits(cap, /*elevated=*/ false)
}

#[test]
fn capability_lattice_blocks_injection_attacks() {
    let cases = corpus();
    let (mut tp, mut fn_, mut tn, mut fp) = (0, 0, 0, 0); // attack-blocked, attack-missed, benign-allowed, benign-blocked
    let mut missed = Vec::new();
    let mut false_pos = Vec::new();

    for c in &cases {
        let blocked = broker_blocks(c);
        match (c.should_block, blocked) {
            (true, true) => tp += 1,
            (true, false) => { fn_ += 1; missed.push(c.name); }
            (false, false) => tn += 1,
            (false, true) => { fp += 1; false_pos.push(c.name); }
        }
    }

    let attacks = tp + fn_;
    let benign = tn + fp;
    let block_rate = tp as f64 / attacks as f64;
    let fpr = fp as f64 / benign as f64;

    // Persist the numbers so the paper/docs cite the test output, not hand-typed figures.
    let report = format!(
        "{{\n  \"attacks\": {attacks},\n  \"benign\": {benign},\n  \"attacks_blocked\": {tp},\n  \
         \"attacks_missed\": {fn_},\n  \"benign_allowed\": {tn},\n  \"benign_blocked\": {fp},\n  \
         \"block_rate\": {block_rate:.4},\n  \"false_positive_rate\": {fpr:.4},\n  \
         \"missed\": {missed:?},\n  \"false_positives\": {false_pos:?}\n}}\n"
    );
    let _ = fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../benchmark/security/capability_redteam.json"),
        &report,
    );
    println!("capability red-team: {report}");

    // The structural guarantee: every side-effecting op on untrusted content is blocked, and no
    // read-only/analysis op is. Both are exact for the lattice as specified.
    assert_eq!(fn_, 0, "an attack was NOT blocked: {missed:?}");
    assert_eq!(fp, 0, "a benign op was wrongly blocked: {false_pos:?}");
    assert!((block_rate - 1.0).abs() < 1e-9);
    assert!(fpr.abs() < 1e-9);
}
