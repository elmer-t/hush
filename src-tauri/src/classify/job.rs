//! The background job that keeps every tag's head up to date. It runs a pass on unlock,
//! then again whenever the vault has changed and been quiet for a while (30 s in the
//! app), so an import or a burst of edits trains once. Same rules as the embedding job:
//! it reaches the vault only through [`Session::with_vault_quiet`] (no activity, so it
//! never postpones auto-lock), never while it computes, and it stops on lock.
//!
//! A head is retrained only when its inputs changed: a SHA-256 over the training
//! version, the ready model, every note vector's id and content hash, and the tag's
//! positive and rejected notes. Heads for another model, and heads of tags that no
//! longer have enough examples, are deleted.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{
    Config, Example, FOLDS, Head, MIN_NEGATIVES, MIN_POSITIVES, REJECTED_WEIGHT, TRAINING_VERSION, UNTAGGED_WEIGHT,
    balance, cross_validate, seed_for, train,
};
use crate::db::{self, StoredHead, new_stored_head};
use crate::embed::job::sleep_while_open;
use crate::session::{Session, SessionError};

/// How long the vault must be unchanged before the app's job retrains.
pub const QUIET: Duration = Duration::from_secs(30);
/// Note vectors read per vault access.
const PAGE: u32 = 500;
/// Held for a whole pass, so a pass asked for on demand (the Suggestions view, after
/// every 10 decisions) and the job's own never train the same heads side by side: the
/// second waits and then finds them current.
static PASSES: Mutex<()> = Mutex::new(());

#[derive(Debug, thiserror::Error)]
pub enum TrainError {
    /// The vault was locked (or another one unlocked): stop quietly.
    #[error("vault is locked")]
    Locked,
    #[error(transparent)]
    Db(#[from] db::Error),
}

impl From<SessionError> for TrainError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Db(e) => Self::Db(e),
            _ => Self::Locked,
        }
    }
}

/// What one pass did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrainPass {
    /// Tags whose head was (re)trained, by name.
    pub trained: Vec<String>,
    /// How long each took, training and cross-validation together, in the same order.
    pub durations: Vec<Duration>,
    /// Heads deleted: another model's, or a tag without enough examples any more.
    pub dropped: usize,
    /// Tags with enough examples whose head was already up to date.
    pub current: usize,
}

/// Runs the job for unlock `generation` until the vault locks. It checks every `poll`
/// whether the vault changed, and trains once it has been unchanged for `quiet`.
/// `report` gets every pass that changed a head.
pub fn run(session: &Session, generation: u64, poll: Duration, quiet: Duration, report: impl Fn(&TrainPass)) {
    let mut seen = None;
    // A pass on unlock, for heads that went stale while locked (or never were trained).
    let mut changed_at = Some(Instant::now() - quiet);
    loop {
        let changes = match session.with_vault_quiet(generation, |v| Ok(v.change_count())) {
            Ok(c) => c,
            Err(_) => return,
        };
        if seen.is_some_and(|s| s != changes) {
            changed_at = Some(Instant::now());
        }
        seen = Some(changes);
        if changed_at.is_some_and(|t| t.elapsed() >= quiet) {
            changed_at = None;
            match pass(session, generation) {
                Ok(p) => {
                    if !p.trained.is_empty() || p.dropped > 0 {
                        report(&p);
                    }
                }
                Err(TrainError::Locked) => return,
                // Not fatal: suggestions fall back to descriptions; the next change retries.
                Err(e) => eprintln!("hush: tag training failed: {e}"),
            }
            // Its own writes aren't a reason to train again.
            match session.with_vault_quiet(generation, |v| Ok(v.change_count())) {
                Ok(c) => seen = Some(c),
                Err(_) => return,
            }
        }
        if !sleep_while_open(session, generation, poll) {
            return;
        }
    }
}

/// One pass: drops heads that shouldn't exist and (re)trains every tag with enough
/// examples whose inputs changed. Each head is written as soon as it is trained.
pub fn pass(session: &Session, generation: u64) -> Result<TrainPass, TrainError> {
    let _one_at_a_time = PASSES.lock().unwrap_or_else(|e| e.into_inner());
    let mut result = TrainPass::default();
    let Some((model, labels, heads)) = session.with_vault_quiet(generation, |v| {
        let Some(model) = v.embedding_model_ready()? else { return Ok(None) };
        Ok(Some((model, v.training_labels()?, v.tag_heads()?)))
    })?
    else {
        return Ok(result);
    };

    let mut ids: Vec<i64> = Vec::new();
    let mut vectors: Vec<Zeroizing<Vec<f32>>> = Vec::new();
    let mut corpus = Sha256::new();
    corpus.update(TRAINING_VERSION.to_le_bytes());
    corpus.update((model.len() as u64).to_le_bytes());
    corpus.update(model.as_bytes());
    let mut after = 0;
    loop {
        let page = session.with_vault_quiet(generation, |v| v.training_vectors(&model, after, PAGE))?;
        for (id, hash, vector) in page.vectors {
            corpus.update(id.to_le_bytes());
            corpus.update(hash);
            ids.push(id);
            vectors.push(vector);
        }
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }
    let corpus = corpus.finalize();
    let index: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let heads: HashMap<&str, &StoredHead> = heads.iter().map(|h| (h.tag.as_str(), h)).collect();

    // Which tags get a head, and which of those are out of date.
    let mut keep = Vec::new();
    let mut stale = Vec::new();
    for labels in &labels {
        let positives: Vec<usize> = labels.positives.iter().filter_map(|id| index.get(id).copied()).collect();
        let rejected: Vec<usize> = labels.rejected.iter().filter_map(|id| index.get(id).copied()).collect();
        let negatives = ids.len() - positives.len();
        if positives.len() < MIN_POSITIVES || negatives < MIN_NEGATIVES {
            continue;
        }
        let mut inputs = Sha256::new();
        inputs.update(corpus);
        inputs.update((labels.tag.len() as u64).to_le_bytes());
        inputs.update(labels.tag.as_bytes());
        for set in [&positives, &rejected] {
            inputs.update((set.len() as u64).to_le_bytes());
            set.iter().for_each(|i| inputs.update(ids[*i].to_le_bytes()));
        }
        let inputs: [u8; 32] = inputs.finalize().into();
        keep.push(labels.tag.clone());
        match heads.get(labels.tag.as_str()) {
            Some(h) if h.model == model && h.inputs == inputs => result.current += 1,
            _ => stale.push((labels.tag.clone(), positives, rejected, negatives, inputs)),
        }
    }
    result.dropped = session.with_vault_quiet(generation, |v| v.retain_tag_heads(&model, &keep))?;

    let config = Config::default();
    let keep_going = || session.is_open(generation);
    for (tag, positives, rejected, negatives, inputs) in stale {
        if !keep_going() {
            return Err(TrainError::Locked);
        }
        let started = Instant::now();
        let mut weight = vec![UNTAGGED_WEIGHT; ids.len()];
        rejected.iter().for_each(|i| weight[*i] = REJECTED_WEIGHT);
        let mut positive = vec![false; ids.len()];
        positives.iter().for_each(|i| positive[*i] = true);
        let examples: Vec<Example> = vectors
            .iter()
            .enumerate()
            .map(|(i, x)| Example { x: x.as_slice(), positive: positive[i], weight: weight[i] })
            .collect();
        // Warm start from the previous head of the same model.
        let start: Option<Head> =
            heads.get(tag.as_str()).filter(|h| h.model == model).map(|h| h.head.clone());
        let mut balanced = examples.clone();
        balance(&mut balanced);
        let head = train(&balanced, start.as_ref(), &config).head;
        let Some(quality) = cross_validate(&examples, FOLDS, seed_for(&tag), Some(&head), &config, &keep_going) else {
            return Err(TrainError::Locked);
        };
        let elapsed = started.elapsed();
        let stored = new_stored_head(&tag, &model, head, (positives.len(), negatives), quality, inputs);
        if !session.with_vault_quiet(generation, |v| v.write_tag_head(&stored))? {
            // The ready model changed meanwhile; the next pass starts over with it.
            return Ok(result);
        }
        result.trained.push(tag);
        result.durations.push(elapsed);
    }
    Ok(result)
}
