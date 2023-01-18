//! The Sync Leaper
mod error;
mod event;
mod leap_activity;
mod leap_state;
mod metrics;
#[cfg(test)]
mod tests;

use std::{sync::Arc, time::Instant};

use datasize::DataSize;
use prometheus::Registry;
use thiserror::Error;
use tracing::{error, info, warn};

use crate::{
    components::{
        fetcher::{self, FetchResult, FetchedData},
        Component,
    },
    effect::{requests::FetcherRequest, EffectBuilder, EffectExt, Effects},
    types::{Chainspec, NodeId, SyncLeap, SyncLeapIdentifier},
    NodeRng,
};
pub(crate) use error::LeapActivityError;
pub(crate) use event::Event;
pub(crate) use leap_state::LeapState;

use metrics::Metrics;

use self::leap_activity::LeapActivity;

const COMPONENT_NAME: &str = "sync_leaper";

#[derive(Clone, Debug, DataSize)]
pub(crate) enum PeerState {
    RequestSent,
    Rejected,
    CouldntFetch,
    Fetched(Box<SyncLeap>),
}

#[derive(Debug)]
enum RegisterLeapAttemptOutcome {
    DoNothing,
    FetchSyncLeapFromPeers(Vec<NodeId>),
}

#[derive(Debug, Error)]
enum Error {
    #[error("fetched a sync leap from storage - should never happen - {0}")]
    FetchedSyncLeapFromStorage(SyncLeapIdentifier),
    #[error("received a sync leap response while no requests were in progress - {0}")]
    UnexpectedSyncLeapResponse(SyncLeapIdentifier),
    #[error("block hash in the response '{actual}' doesn't match the one requested '{expected}'")]
    SyncLeapIdentifierMismatch {
        expected: SyncLeapIdentifier,
        actual: SyncLeapIdentifier,
    },
    #[error(
        "received a sync leap response from an unknown peer - {peer} - {sync_leap_identifier}"
    )]
    ResponseFromUnknownPeer {
        peer: NodeId,
        sync_leap_identifier: SyncLeapIdentifier,
    },
}

#[derive(Debug, DataSize)]
pub(crate) struct SyncLeaper {
    leap_activity: Option<LeapActivity>,
    chainspec: Arc<Chainspec>,
    #[data_size(skip)]
    metrics: Metrics,
}

impl SyncLeaper {
    pub(crate) fn new(
        chainspec: Arc<Chainspec>,
        registry: &Registry,
    ) -> Result<Self, prometheus::Error> {
        Ok(SyncLeaper {
            leap_activity: None,
            chainspec,
            metrics: Metrics::new(registry)?,
        })
    }

    // called from Reactor control logic to scrape results
    pub(crate) fn leap_status(&mut self) -> LeapState {
        match &self.leap_activity {
            None => LeapState::Idle,
            Some(activity) => {
                let result = activity.status();
                if result.active() == false {
                    match result {
                        LeapState::Received { .. } | LeapState::Failed { .. } => {
                            self.metrics
                                .sync_leap_duration
                                .observe(activity.leap_start().elapsed().as_secs_f64());
                        }
                        LeapState::Idle | LeapState::Awaiting { .. } => {
                            // should be unreachable
                            error!(status = %result, ?activity, "sync leaper has inconsistent status");
                        }
                    }
                    self.leap_activity = None;
                }
                result
            }
        }
    }

    fn register_leap_attempt(
        &mut self,
        sync_leap_identifier: SyncLeapIdentifier,
        peers_to_ask: Vec<NodeId>,
    ) -> RegisterLeapAttemptOutcome {
        info!(%sync_leap_identifier, "registering leap attempt");
        if peers_to_ask.is_empty() {
            error!("tried to start fetching a sync leap without peers to ask");
            return RegisterLeapAttemptOutcome::DoNothing;
        }
        if let Some(leap_activity) = self.leap_activity.as_mut() {
            if leap_activity.sync_leap_identifier() != &sync_leap_identifier {
                error!(
                    current_sync_leap_identifier = %leap_activity.sync_leap_identifier(),
                    requested_sync_leap_identifier = %sync_leap_identifier,
                    "tried to start fetching a sync leap for a different sync_leap_identifier"
                );
                return RegisterLeapAttemptOutcome::DoNothing;
            }

            let peers_not_asked_yet: Vec<_> = peers_to_ask
                .iter()
                .filter_map(|peer| leap_activity.register_peer(*peer))
                .collect();

            return if peers_not_asked_yet.is_empty() {
                RegisterLeapAttemptOutcome::DoNothing
            } else {
                RegisterLeapAttemptOutcome::FetchSyncLeapFromPeers(peers_not_asked_yet)
            };
        }

        self.leap_activity = Some(LeapActivity::new(
            sync_leap_identifier,
            peers_to_ask
                .iter()
                .map(|peer| (*peer, PeerState::RequestSent))
                .collect(),
            Instant::now(),
        ));
        RegisterLeapAttemptOutcome::FetchSyncLeapFromPeers(peers_to_ask)
    }

    fn fetch_received(
        &mut self,
        sync_leap_identifier: SyncLeapIdentifier,
        fetch_result: FetchResult<SyncLeap>,
    ) -> Result<(), Error> {
        let leap_activity = match &mut self.leap_activity {
            Some(leap_activity) => leap_activity,
            None => {
                // warn!(
                //     %sync_leap_identifier,
                //     "received a sync leap response while no requests were in progress"
                // );
                panic!("1");
                return Err(Error::UnexpectedSyncLeapResponse(sync_leap_identifier));
            }
        };

        if leap_activity.sync_leap_identifier() != &sync_leap_identifier {
            // warn!(
            //     requested_hash=%leap_activity.sync_leap_identifier(),
            //     response_hash=%sync_leap_identifier,
            //     "block hash in the response doesn't match the one requested"
            // );
            panic!("2");
            return Err(Error::SyncLeapIdentifierMismatch {
                actual: sync_leap_identifier,
                expected: *leap_activity.sync_leap_identifier(),
            });
        }

        match fetch_result {
            Ok(FetchedData::FromStorage { .. }) => {
                //error!(%sync_leap_identifier, "fetched a sync leap from storage - should never happen");
                return Err(Error::FetchedSyncLeapFromStorage(sync_leap_identifier));
            }
            Ok(FetchedData::FromPeer { item, peer, .. }) => {
                let peer_state = match leap_activity.peers_mut().get_mut(&peer) {
                    Some(state) => state,
                    None => {
                        // warn!(
                        //     ?peer,
                        //     %sync_leap_identifier,
                        //     "received a sync leap response from an unknown peer"
                        // );
                        panic!("4");
                        return Err(Error::ResponseFromUnknownPeer {
                            peer,
                            sync_leap_identifier,
                        });
                    }
                };
                *peer_state = PeerState::Fetched(Box::new(*item));
                self.metrics.sync_leap_fetched_from_peer.inc();
                panic!("5");
            }
            Err(fetcher::Error::Rejected { peer, .. }) => {
                let peer_state = match leap_activity.peers_mut().get_mut(&peer) {
                    Some(state) => state,
                    None => {
                        // warn!(
                        //     ?peer,
                        //     %sync_leap_identifier,
                        //     "received a sync leap response from an unknown peer"
                        // );
                        panic!("6");
                        return Err(Error::ResponseFromUnknownPeer {
                            peer,
                            sync_leap_identifier,
                        });
                    }
                };
                info!(%peer, %sync_leap_identifier, "peer rejected our request for a sync leap");
                *peer_state = PeerState::Rejected;
                self.metrics.sync_leap_rejected_by_peer.inc();
                panic!("7");
            }
            Err(error) => {
                let peer = error.peer();
                info!(?error, %peer, %sync_leap_identifier, "failed to fetch a sync leap from peer");
                let peer_state = match leap_activity.peers_mut().get_mut(peer) {
                    Some(state) => state,
                    None => {
                        // warn!(
                        //     ?peer,
                        //     %sync_leap_identifier,
                        //     "received a sync leap response from an unknown peer"
                        // );
                        panic!("8");
                        return Err(Error::ResponseFromUnknownPeer {
                            peer: *peer,
                            sync_leap_identifier,
                        });
                    }
                };
                *peer_state = PeerState::CouldntFetch;
                self.metrics.sync_leap_cant_fetch.inc();
                panic!("9");
            }
        }
        panic!("10");

        Ok(())
    }
}

impl<REv> Component<REv> for SyncLeaper
where
    REv: From<FetcherRequest<SyncLeap>> + Send,
{
    type Event = Event;

    fn handle_event(
        &mut self,
        effect_builder: EffectBuilder<REv>,
        _rng: &mut NodeRng,
        event: Self::Event,
    ) -> Effects<Self::Event> {
        match event {
            Event::AttemptLeap {
                sync_leap_identifier,
                peers_to_ask,
            } => match self.register_leap_attempt(sync_leap_identifier, peers_to_ask) {
                RegisterLeapAttemptOutcome::DoNothing => Effects::new(),
                RegisterLeapAttemptOutcome::FetchSyncLeapFromPeers(peers) => {
                    let mut effects = Effects::new();
                    peers.into_iter().for_each(|peer| {
                        effects.extend(
                            effect_builder
                                .fetch::<SyncLeap>(
                                    sync_leap_identifier,
                                    peer,
                                    self.chainspec.clone(),
                                )
                                .event(move |fetch_result| Event::FetchedSyncLeapFromPeer {
                                    sync_leap_identifier,
                                    fetch_result,
                                }),
                        )
                    });
                    effects
                }
            },
            Event::FetchedSyncLeapFromPeer {
                sync_leap_identifier,
                fetch_result,
            } => {
                self.fetch_received(sync_leap_identifier, fetch_result);
                Effects::new()
            }
        }
    }

    fn name(&self) -> &str {
        COMPONENT_NAME
    }
}

#[cfg(test)]
impl SyncLeaper {
    fn peers(&self) -> Option<Vec<(NodeId, PeerState)>> {
        self.leap_activity
            .as_ref()
            .and_then(|leap_activity| {
                let peers = leap_activity.peers();
                if leap_activity.peers().is_empty() {
                    None
                } else {
                    Some(peers.clone())
                }
            })
            .map(|peers| peers.into_iter().collect::<Vec<_>>())
    }
}
