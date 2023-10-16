use std::{path::PathBuf, str::FromStr, sync::Arc};

use action::action_core::ActionConfig;
use clap::Parser;
use ethers_providers::{Provider, Ws};
use ethers_signers::LocalWallet;
use guard_network::{config::SecretKey, NetworkConfig};
use hex_literal::hex;
use reth_primitives::{mainnet_nodes, NodeRecord, PeerId, H512};
use sim::{lru_db::RevmLRU, spawn_revm_sim};
use stale_guard::{Guard, SubmissionServerConfig};
use tokio::runtime::Runtime;
use url::Url;

#[derive(Debug, Parser)]
pub struct Args {
    // #[arg(long, value_name = "FILE")]
    // pub staking_secret_key:   PathBuf,
    // #[arg(long, value_name = "FILE")]
    // pub bundle_key:           PathBuf,
    // #[arg(long, value_name = "FILE")]
    // pub submission_key:       PathBuf,
    // #[arg(long, value_delimiter = ',')]
    // pub bootnodes:            Option<Vec<NodeRecord>>,
    // #[arg(long, value_delimiter = ',')]
    // pub builder_submissions:  Option<Vec<Url>>,
    // #[arg(long)]
    // pub bundle_sim_url:       Option<Url>,
    #[arg(long, default_value = "false")]
    pub enable_subscriptions: bool,
    #[arg(long)]
    pub full_node:            PathBuf,
    #[arg(long)]
    pub full_node_ws:         String
}

impl Args {
    pub fn run(self, rt: Runtime) -> anyhow::Result<()> {
        reth_tracing::init_test_tracing();

        let fake_key =
            SecretKey::from_str("ad21c16051f74f24b3fbad57b0010d98bfef20441c84ee5a872133f19f807fc4")
                .unwrap();
        let fake_pub_key: Vec<u8>= hex!("04b80d553719877f1aac5f60b816300cd26ba35bf9275e5105400aa5edfc0b69256f920019187647446ecd24fbc8a7714ef580b76ab14a8185a3370426fa6df9d8").to_vec();
        let fake_pub_key = fake_pub_key.as_slice();

        let fake_pub_key: &[u8; 64] =
            unsafe { &*(fake_pub_key as *const _ as *mut [u8]).cast() as &[u8; 64] };

        let middleware = Box::leak(Box::new(Provider::new(
            rt.block_on(Ws::connect(self.full_node_ws.parse::<Url>()?))?
        )));

        let db_path = self.full_node.as_ref();
        let db = Arc::new(reth_db::mdbx::Env::<reth_db::mdbx::WriteMap>::open(
            db_path,
            reth_db::mdbx::EnvKind::RO,
            None
        )?);
        let revm_lru = RevmLRU::new(9999999, db);

        let sim = spawn_revm_sim(revm_lru).unwrap();
        let edsca_key = LocalWallet::from_str(
            "ad21c16051f74f24b3fbad57b0010d98bfef20441c84ee5a872133f19f807fc4"
        )?;

        let fake_bundle = LocalWallet::new(&mut rand::thread_rng());
        let network_config = NetworkConfig::new(fake_key, fake_pub_key.into());
        let action_config = ActionConfig { simulator: sim, edsca_key, bundle_key: fake_bundle };

        let fake_addr = "127.0.0.1:6969".parse()?;
        let server_config = SubmissionServerConfig {
            addr:                fake_addr,
            cors_domains:        "*".into(),
            allow_subscriptions: self.enable_subscriptions
        };
        println!("spawning guard");

        let guard =
            rt.block_on(Guard::new(middleware, network_config, action_config, server_config))?;
        rt.block_on(guard);

        Ok(())
    }
}

fn main() -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    Args::parse().run(rt)?;

    Ok(())
}
