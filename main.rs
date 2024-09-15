// src/main.rs

use futures::prelude::*;
use libp2p::{
    core::upgrade,
    identity,
    kad::{
        record::store::MemoryStore, record::store::RecordStore, AddProviderOk, GetProvidersOk,
        Kademlia, KademliaConfig, KademliaEvent, PeerRecord, PutRecordOk, QueryResult, Quorum,
        Record,
    },
    mplex, noise,
    swarm::{NetworkBehaviourEventProcess, Swarm, SwarmBuilder, SwarmEvent},
    tcp::TokioTcpConfig,
    Multiaddr, NetworkBehaviour, PeerId, Transport,
};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    error::Error,
    time::{Duration, Instant},
};
use tokio::time::{interval, sleep};

/// A simple reputation system where each peer has a score.
struct ReputationSystem {
    scores: HashMap<PeerId, i32>,
}

impl ReputationSystem {
    fn new() -> Self {
        ReputationSystem {
            scores: HashMap::new(),
        }
    }

    /// Increase the reputation score of a peer.
    fn increase(&mut self, peer: &PeerId) {
        *self.scores.entry(*peer).or_insert(0) += 1;
    }

    /// Decrease the reputation score of a peer.
    fn decrease(&mut self, peer: &PeerId) {
        *self.scores.entry(*peer).or_insert(0) -= 1;
    }

    /// Get the reputation score of a peer.
    fn get(&self, peer: &PeerId) -> i32 {
        *self.scores.get(peer).unwrap_or(&0)
    }
}

/// Our custom record value structure.
#[derive(Serialize, Deserialize, Debug)]
struct MyRecordValue {
    data: String,
}

impl MyRecordValue {
    fn new(data: String) -> Self {
        MyRecordValue { data }
    }
}

/// Define the network behaviour combining Kademlia and the reputation system.
#[derive(NetworkBehaviour)]
#[behaviour(event_process = true)]
struct MyBehaviour {
    kademlia: Kademlia<MemoryStore>,
    #[behaviour(ignore)]
    reputation: ReputationSystem,
}

impl NetworkBehaviourEventProcess<KademliaEvent> for MyBehaviour {
    fn inject_event(&mut self, event: KademliaEvent) {
        match event {
            // Handle inbound requests.
            KademliaEvent::InboundRequest { request } => {
                let peer = request.source;
                info!("Received request from {:?}", peer);
                self.reputation.increase(&peer);
            }
            // Handle outbound query completions.
            KademliaEvent::OutboundQueryCompleted { result, .. } => match result {
                QueryResult::GetClosestPeers(Ok(ok)) => {
                    for peer in &ok.peers {
                        info!("Found peer {:?}", peer);
                        self.reputation.increase(peer);
                    }
                }
                QueryResult::GetClosestPeers(Err(err)) => {
                    warn!("Failed to find peers: {:?}", err);
                    // Decrease reputation of peers involved.
                }
                QueryResult::GetRecord(Ok(ok)) => {
                    for PeerRecord { record, .. } in ok.records {
                        if let Ok(value) = serde_json::from_slice::<MyRecordValue>(&record.value) {
                            info!(
                                "Received record for key {:?}: {:?}",
                                String::from_utf8_lossy(&record.key),
                                value
                            );
                        } else {
                            warn!("Failed to deserialize record value.");
                        }
                    }
                }
                QueryResult::GetRecord(Err(err)) => {
                    warn!("Failed to get record: {:?}", err);
                }
                QueryResult::PutRecord(Ok(PutRecordOk { key })) => {
                    info!("Successfully put record with key {:?}", key);
                }
                QueryResult::PutRecord(Err(err)) => {
                    warn!("Failed to put record: {:?}", err);
                }
                QueryResult::Bootstrap(Ok(_)) => {
                    info!("Bootstrap successful");
                }
                QueryResult::Bootstrap(Err(err)) => {
                    warn!("Bootstrap failed: {:?}", err);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    env_logger::init();

    // Generate a random PeerId.
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    info!("Local peer id: {:?}", local_peer_id);

    // Set up the transport.
    let noise_keys = noise::Keypair::<noise::X25519Spec>::new()
        .into_authentic(&local_key)
        .expect("Signing libp2p-noise static DH keypair failed.");
    let transport = TokioTcpConfig::new()
        .nodelay(true)
        .upgrade(upgrade::Version::V1)
        .authenticate(noise::NoiseConfig::xx(noise_keys).into_authenticated())
        .multiplex(mplex::MplexConfig::new())
        .boxed();

    // Create a Kademlia behaviour with advanced configuration.
    let store = MemoryStore::new(local_peer_id);
    let mut cfg = KademliaConfig::default();
    cfg.set_kbucket_inserts(libp2p::kad::KademliaBucketInserts::Manual);
    cfg.set_query_timeout(Duration::from_secs(10));
    let mut kademlia = Kademlia::with_config(local_peer_id, store, cfg);

    // Add bootstrap nodes if available.
    // For example:
    // let bootstrap_peer_id = PeerId::from_str("12D3KooW...")?;
    // let bootstrap_addr: Multiaddr = "/ip4/127.0.0.1/tcp/8080".parse()?;
    // kademlia.add_address(&bootstrap_peer_id, bootstrap_addr);

    // Combine Kademlia with the reputation system.
    let behaviour = MyBehaviour {
        kademlia,
        reputation: ReputationSystem::new(),
    };

    let mut swarm = SwarmBuilder::new(transport, behaviour, local_peer_id)
        .executor(Box::new(|fut| {
            tokio::spawn(fut);
        }))
        .build();

    // Listen on all interfaces and a random OS-assigned port.
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Start periodic tasks.
    let mut refresh_interval = interval(Duration::from_secs(60));

    // Start the swarm.
    loop {
        tokio::select! {
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(event) => {
                    // Events are handled in the inject_event method.
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    info!("Listening on {:?}", address);
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    info!("Connected to {:?}", peer_id);
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    info!("Disconnected from {:?}", peer_id);
                }
                _ => {}
            },
            _ = sleep(Duration::from_secs(10)) => {
                // Periodically print out the reputation scores.
                info!("Current reputations:");
                for (peer, score) in &swarm.behaviour().reputation.scores {
                    info!("{:?} => {}", peer, score);
                }

                // Periodically search for more peers.
                let random_peer = PeerId::random();
                swarm.behaviour_mut().kademlia.get_closest_peers(random_peer);
            },
            _ = refresh_interval.tick() => {
                // Refresh buckets and republish records.
                info!("Refreshing routing table and republishing records.");
                swarm.behaviour_mut().kademlia.bootstrap()?;

                // Republish all records.
                for record in swarm.behaviour().kademlia.store().records() {
                    let key = record.key.clone();
                    swarm.behaviour_mut().kademlia.put_record(record.into_owned(), Quorum::One)?;
                    info!("Republishing record with key {:?}", key);
                }
            }
        }
    }
}
