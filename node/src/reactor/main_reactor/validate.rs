use std::time::Duration;
use tracing::{debug, info};

use crate::{
    effect::{EffectBuilder, Effects},
    reactor,
    reactor::main_reactor::{MainEvent, MainReactor},
    NodeRng,
};

pub(super) enum ValidateInstruction {
    Do(Duration, Effects<MainEvent>),
    CheckLater(String, Duration),
    NonSwitchBlock,
    KeepUp,
    ShutdownForUpgrade,
    Fatal(String),
}

impl MainReactor {
    pub(super) fn validate_instruction(
        &mut self,
        effect_builder: EffectBuilder<MainEvent>,
        rng: &mut NodeRng,
    ) -> ValidateInstruction {
        if self.switch_block.is_none() {
            // validate status is only checked at switch blocks
            return ValidateInstruction::NonSwitchBlock;
        }

        if self.should_shutdown_for_upgrade() {
            return ValidateInstruction::ShutdownForUpgrade;
        }

        match self.create_required_eras(effect_builder, rng) {
            Ok(Some(effects)) => {
                let last_progress = self.consensus.last_progress();
                if last_progress > self.last_progress {
                    self.last_progress = last_progress;
                }
                if effects.is_empty() {
                    ValidateInstruction::CheckLater(
                        "consensus state is up to date".to_string(),
                        self.control_logic_default_delay.into(),
                    )
                } else {
                    ValidateInstruction::Do(Duration::ZERO, effects)
                }
            }
            Ok(None) => ValidateInstruction::KeepUp,
            Err(msg) => ValidateInstruction::Fatal(msg),
        }
    }

    pub(super) fn create_required_eras(
        &mut self,
        effect_builder: EffectBuilder<MainEvent>,
        rng: &mut NodeRng,
    ) -> Result<Option<Effects<MainEvent>>, String> {
        let highest_switch_block_header = match self.recent_switch_block_headers.last() {
            None => {
                debug!("create_required_eras: recent_switch_block_headers is empty");
                return Ok(None);
            }
            Some(header) => header,
        };
        debug!(
            "highest_switch_block_header: {} - {}",
            highest_switch_block_header.era_id(),
            highest_switch_block_header.block_hash(),
        );

        if let Some(current_era) = self.consensus.current_era() {
            debug!("consensus current_era: {}", current_era.value());
            if highest_switch_block_header.next_block_era_id() <= current_era {
                return Ok(Some(Effects::new()));
            }
        }

        let highest_era_weights = match highest_switch_block_header.next_era_validator_weights() {
            None => {
                return Err(format!(
                    "highest switch block has no era end: {}",
                    highest_switch_block_header
                ));
            }
            Some(weights) => weights,
        };
        if !highest_era_weights.contains_key(self.consensus.public_key()) {
            debug!("highest_era_weights does not contain signing_public_key");
            return Ok(None);
        }

        // get local tip if we have it, otherwise
        // highest switch block
        let from_height = match self.block_accumulator.local_tip() {
            Some(tip) => tip,
            None => highest_switch_block_header.height(),
        };

        if !self.deploy_buffer.have_full_ttl_of_deploys(from_height) {
            info!("currently have insufficient deploy TTL awareness to safely participate in consensus");
            return Ok(None);
        }

        let era_id = highest_switch_block_header.era_id();
        if self.upgrade_watcher.should_upgrade_after(era_id) {
            debug!(%era_id, "upgrade required after given era");
            return Ok(None);
        }

        let create_required_eras = self.consensus.create_required_eras(
            effect_builder,
            rng,
            &self.recent_switch_block_headers,
        );
        if create_required_eras.is_some() {
            info!("will attempt to create required eras for consensus");
        }

        Ok(
            create_required_eras
                .map(|effects| reactor::wrap_effects(MainEvent::Consensus, effects)),
        )
    }
}
