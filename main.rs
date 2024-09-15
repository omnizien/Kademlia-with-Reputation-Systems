// src/main.rs

use futures::prelude::*;
use libp2p::{
    identity,
    kad::{
        record::store::MemoryStore, GetClosestPeersOk, Kademlia, KademliaConfig, KademliaEvent,
        QueryId, QueryResult,
    },
    swarm::{NetworkBehaviourEventProcess, Swarm, SwarmEvent},
    Multiaddr, NetworkBehaviour, PeerId,
};
use std::{collections::HashMap, error::Error, time::Duration};
use tokio::time::{sleep, timeout};

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
            // Handle events where we receive a request from a peer.
            KademliaEvent::InboundRequest { request } => {
                let peer = request.source;
                println!("Received request from {:?}", peer);
                self.reputation.increase(&peer);
            }
            // Handle the completion of outbound queries.
            KademliaEvent::OutboundQueryCompleted { result, .. } => match result {
                QueryResult::GetClosestPeers(Ok(GetClosestPeersOk { peers, .. })) => {
                    for peer in &peers {
                        println!("Found peer {:?}", peer);
                        self.reputation.increase(peer);
                    }
                }
                QueryResult::GetClosestPeers(Err(err)) => {
                    let peer = err.peer;
                    println!("Failed to find peer {:?}: {:?}", peer, err.error);
                    self.reputation.decrease(&peer);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Generate a random PeerId.
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    println!("Local peer id: {:?}", local_peer_id);

    // Set up the transport.
    let transport = libp2p::development_transport(local_key.clone()).await?;

    // Create a Kademlia behaviour.
    let store = MemoryStore::new(local_peer_id);
    let mut cfg = KademliaConfig::default();
    cfg.set_query_timeout(Duration::from_secs(5));
    let mut kademlia = Kademlia::with_config(local_peer_id, store, cfg);

    // Add bootstrap nodes if available (replace with actual addresses if you have them).
    // For example:
    // let bootstrap_peer_id = PeerId::from_str("Qm...")?;
    // let bootstrap_addr: Multiaddr = "/ip4/127.0.0.1/tcp/8080".parse()?;
    // kademlia.add_address(&bootstrap_peer_id, bootstrap_addr);

    // Combine Kademlia with the reputation system.
    let mut behaviour = MyBehaviour {
        kademlia,
        reputation: ReputationSystem::new(),
    };

    let mut swarm = Swarm::new(transport, behaviour, local_peer_id);

    // Listen on all interfaces and a random OS-assigned port.
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    // Start the swarm.
    loop {
        tokio::select! {
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(event) => {
                    // Events are handled in the inject_event method.
                }
                SwarmEvent::NewListenAddr { address, .. } => {
                    println!("Listening on {:?}", address);
                }
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    println!("Connected to {:?}", peer_id);
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    println!("Disconnected from {:?}", peer_id);
                }
                _ => {}
            },
            _ = sleep(Duration::from_secs(10)) => {
                // Periodically print out the reputation scores.
                println!("Current reputations:");
                for (peer, score) in &swarm.behaviour().reputation.scores {
                    println!("{:?} => {}", peer, score);
                }

                // Periodically search for more peers.
                let random_peer = PeerId::random();
                swarm.behaviour_mut().kademlia.get_closest_peers(random_peer);
            }
        }
    }
}
