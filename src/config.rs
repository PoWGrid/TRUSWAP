use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayMode {
    Blazing,
    UltraBlazing,
}

impl Default for RelayMode {
    fn default() -> Self {
        Self::Blazing
    }
}

pub const BASE_MAINNET_RPC: &str = "https://mainnet.base.org";
pub const BASE_MAINNET_WSS: &str = "wss://mainnet.base.org";
pub const BASE_MAINNET_USDC: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"; // Circle Official USDC Base Mainnet
pub const BASE_MAINNET_HTLC_DEFAULT: &str = "0x812b611208297eb7A2b153BECA8b2AF47C7614DF";

pub const BASE_SEPOLIA_RPC: &str = "https://sepolia.base.org";
pub const BASE_SEPOLIA_WSS: &str = "wss://sepolia.base.org";
pub const BASE_SEPOLIA_USDC: &str = "0x036CbD53842c5426634e7929541eC2318f3dCF7e";
pub const BASE_SEPOLIA_HTLC: &str = "0xF2e70fbd7Da79d188f805e61e8587AACD49772a5";

pub const BASE_MAINNET_FALLBACK_RPCS: &[&str] = &[
    "https://mainnet.base.org",
    "https://base-rpc.publicnode.com",
    "https://1rpc.io/base",
    "https://base.llamarpc.com",
    "https://base.meowrpc.com",
];

pub const BASE_SEPOLIA_FALLBACK_RPCS: &[&str] = &[
    "https://sepolia.base.org",
    "https://base-sepolia-rpc.publicnode.com",
    "https://gateway.tenderly.co/public/base-sepolia",
    "https://base-sepolia.blockpi.network/v1/rpc/public",
];

pub const DEFAULT_RELAY_NODE_URL: &str = "wss://tru.powgrid.xyz/orderbook";

pub fn normalize_relay_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return DEFAULT_RELAY_NODE_URL.to_string();
    }
    let with_scheme = if trimmed.starts_with("https://") {
        format!("wss://{}", &trimmed["https://".len()..])
    } else if trimmed.starts_with("http://") {
        format!("ws://{}", &trimmed["http://".len()..])
    } else if trimmed.starts_with("wss://") || trimmed.starts_with("ws://") {
        trimmed.to_string()
    } else {
        format!("ws://{}", trimmed)
    };

    let scheme_end = with_scheme.find("://").map(|pos| pos + 3).unwrap_or(0);
    let rest = &with_scheme[scheme_end..];
    if let Some(slash_idx) = rest.find('/') {
        let path = &rest[slash_idx..];
        if path.is_empty() || path == "/" {
            format!("{}/orderbook", with_scheme.trim_end_matches('/'))
        } else {
            with_scheme
        }
    } else {
        format!("{}/orderbook", with_scheme)
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: String,
    pub is_testnet: bool,
    pub base_rpc_url: String,
    pub base_wss_url: String,
    pub base_htlc_contract: String,
    pub base_usdc_contract: String,
    pub tru_seeds: Vec<String>,
    pub data_dir: PathBuf,
    pub is_relay: bool,
    pub relay_mode: RelayMode,
    pub max_relay_clients: usize,
    pub min_eth_gas: f64,
    pub resync_from_genesis: bool,
    pub relay_node_url: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:8899".to_string(),
            is_testnet: false, // Default is Mainnet!
            base_rpc_url: BASE_MAINNET_RPC.to_string(),
            base_wss_url: BASE_MAINNET_WSS.to_string(),
            base_htlc_contract: BASE_MAINNET_HTLC_DEFAULT.to_string(),
            base_usdc_contract: BASE_MAINNET_USDC.to_string(),
            tru_seeds: vec![
                "137.184.68.43:21833".to_string(),
                "193.181.216.10:21833".to_string(),
            ],
            data_dir: PathBuf::from("./data"),
            is_relay: false,
            relay_mode: RelayMode::Blazing,
            max_relay_clients: 5000,
            min_eth_gas: 0.0001,
            resync_from_genesis: false,
            relay_node_url: DEFAULT_RELAY_NODE_URL.to_string(),
        }
    }
}

impl Config {
    pub fn chain_id(&self) -> crate::chains::types::ChainId {
        if self.is_testnet {
            crate::chains::types::ChainId::BaseSepolia
        } else {
            crate::chains::types::ChainId::BaseMainnet
        }
    }

    pub fn explorer_url(&self) -> &'static str {
        self.chain_id().default_explorer()
    }

    pub fn network_name(&self) -> &'static str {
        self.chain_id().display_name()
    }

    pub fn fallback_rpcs(&self) -> &'static [&'static str] {
        if self.is_testnet {
            BASE_SEPOLIA_FALLBACK_RPCS
        } else {
            BASE_MAINNET_FALLBACK_RPCS
        }
    }

    pub fn set_testnet(&mut self, testnet: bool) {
        self.is_testnet = testnet;
        if testnet {
            self.base_rpc_url = BASE_SEPOLIA_RPC.to_string();
            self.base_wss_url = BASE_SEPOLIA_WSS.to_string();
            self.base_htlc_contract = BASE_SEPOLIA_HTLC.to_string();
            self.base_usdc_contract = BASE_SEPOLIA_USDC.to_string();
        } else {
            self.base_rpc_url = BASE_MAINNET_RPC.to_string();
            self.base_wss_url = BASE_MAINNET_WSS.to_string();
            self.base_htlc_contract = BASE_MAINNET_HTLC_DEFAULT.to_string();
            self.base_usdc_contract = BASE_MAINNET_USDC.to_string();
        }
    }

    pub fn parse() -> Self {
        Self::parse_from_args(std::env::args())
    }

    pub fn parse_from_args<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        let mut cfg = Self::default();

        // 1. Detect testnet mode first from args or env
        let args: Vec<String> = args.into_iter().map(Into::into).collect();
        let has_testnet_arg = args.iter().any(|a| a == "--testnet");
        let has_testnet_env = std::env::var("TRUSWAP_TESTNET").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);

        if has_testnet_arg || has_testnet_env {
            cfg.set_testnet(true);
        }

        // 2. Parse environment variables (can override testnet/mainnet defaults)
        if let Ok(v) = std::env::var("BIND_ADDR") {
            cfg.bind_addr = v;
        }
        if let Ok(v) = std::env::var("TRUSWAP_DEV") {
            if v == "1" || v.eq_ignore_ascii_case("true") {
                cfg.bind_addr = "0.0.0.0:8899".to_string();
            }
        }
        if let Ok(v) = std::env::var("TRU_SEEDS") {
            cfg.tru_seeds = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Ok(v) = std::env::var("BASE_RPC_URL") {
            cfg.base_rpc_url = v;
        }
        if let Ok(v) = std::env::var("BASE_WSS_URL") {
            cfg.base_wss_url = v;
        }
        if let Ok(v) = std::env::var("BASE_HTLC_CONTRACT") {
            cfg.base_htlc_contract = v;
        }
        if let Ok(v) = std::env::var("BASE_USDC_CONTRACT") {
            cfg.base_usdc_contract = v;
        }
        if let Ok(v) = std::env::var("DATA_DIR") {
            cfg.data_dir = PathBuf::from(v);
        }
        if let Ok(v) = std::env::var("TRUSWAP_RELAY") {
            cfg.is_relay = v == "1" || v.eq_ignore_ascii_case("true");
        }
        if let Ok(v) = std::env::var("TRUSWAP_RELAY_NODE") {
            cfg.relay_node_url = normalize_relay_url(&v);
        }
        if let Ok(v) = std::env::var("RELAY_NODE_URL") {
            cfg.relay_node_url = normalize_relay_url(&v);
        }
        if let Ok(v) = std::env::var("RELAY_MODE") {
            if v.eq_ignore_ascii_case("ultrablazing") {
                cfg.relay_mode = RelayMode::UltraBlazing;
                cfg.max_relay_clients = 100_000;
            } else {
                cfg.relay_mode = RelayMode::Blazing;
                cfg.max_relay_clients = 5_000;
            }
        }

        if let Ok(v) = std::env::var("RESYNC") {
            cfg.resync_from_genesis = v == "1" || v.eq_ignore_ascii_case("true");
        }

        // 3. Parse command line arguments (precedence over env)
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--dev" => {
                    cfg.bind_addr = "0.0.0.0:8899".to_string();
                }
                "--testnet" => {
                    cfg.set_testnet(true);
                }
                "--mainnet" => {
                    cfg.set_testnet(false);
                }
                "--resync" => {
                    cfg.resync_from_genesis = true;
                }
                "--relay" | "--truswap-relay" => {
                    cfg.is_relay = true;
                }
                "--relay-node" | "--relay-url" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.relay_node_url = normalize_relay_url(&args[i]);
                    }
                }
                "--ultrablazing" => {
                    cfg.relay_mode = RelayMode::UltraBlazing;
                    cfg.max_relay_clients = 100_000;
                }
                "--blazing" => {
                    cfg.relay_mode = RelayMode::Blazing;
                    cfg.max_relay_clients = 5_000;
                }
                "--mode" => {
                    if i + 1 < args.len() {
                        i += 1;
                        if args[i].eq_ignore_ascii_case("ultrablazing") {
                            cfg.relay_mode = RelayMode::UltraBlazing;
                            cfg.max_relay_clients = 100_000;
                        } else {
                            cfg.relay_mode = RelayMode::Blazing;
                            cfg.max_relay_clients = 5_000;
                        }
                    }
                }
                "--relay-bind" | "--bind" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.bind_addr = args[i].clone();
                    }
                }
                "--base-rpc" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.base_rpc_url = args[i].clone();
                    }
                }
                "--base-wss" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.base_wss_url = args[i].clone();
                    }
                }
                "--base-htlc" | "--htlc" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.base_htlc_contract = args[i].clone();
                    }
                }
                "--base-usdc" | "--usdc" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.base_usdc_contract = args[i].clone();
                    }
                }
                "--max-clients" => {
                    if i + 1 < args.len() {
                        i += 1;
                        if let Ok(num) = args[i].parse() {
                            cfg.max_relay_clients = num;
                        }
                    }
                }
                "--min-eth-gas" => {
                    if i + 1 < args.len() {
                        i += 1;
                        if let Ok(num) = args[i].parse() {
                            cfg.min_eth_gas = num;
                        }
                    }
                }
                "--data-dir" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.data_dir = PathBuf::from(&args[i]);
                    }
                }
                "--tru-seeds" => {
                    if i + 1 < args.len() {
                        i += 1;
                        cfg.tru_seeds = args[i]
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect();
                    }
                }
                _ => {}
            }
            i += 1;
        }

        cfg
    }

    // Retain backward compatibility
    pub fn from_env() -> Self {
        Self::parse()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_base_mainnet() {
        let cfg = Config::parse_from_args(vec!["truswap"]);
        assert!(!cfg.is_testnet);
        assert_eq!(cfg.chain_id(), crate::chains::types::ChainId::BaseMainnet);
        assert_eq!(cfg.chain_id().as_u64(), 8453);
        assert_eq!(cfg.network_name(), "Base");
        assert_eq!(cfg.explorer_url(), "https://basescan.org");
        assert_eq!(cfg.base_usdc_contract, BASE_MAINNET_USDC);
        assert_eq!(cfg.base_rpc_url, BASE_MAINNET_RPC);
        assert_eq!(cfg.relay_node_url, DEFAULT_RELAY_NODE_URL);
    }

    #[test]
    fn test_flag_testnet_switches_to_sepolia() {
        let cfg = Config::parse_from_args(vec!["truswap", "--testnet"]);
        assert!(cfg.is_testnet);
        assert_eq!(cfg.chain_id(), crate::chains::types::ChainId::BaseSepolia);
        assert_eq!(cfg.chain_id().as_u64(), 84532);
        assert_eq!(cfg.network_name(), "Base");
        assert_eq!(cfg.explorer_url(), "https://sepolia.basescan.org");
        assert_eq!(cfg.base_usdc_contract, BASE_SEPOLIA_USDC);
        assert_eq!(cfg.base_htlc_contract, BASE_SEPOLIA_HTLC);
        assert_eq!(cfg.base_rpc_url, BASE_SEPOLIA_RPC);
    }

    #[test]
    fn test_mainnet_overrides_testnet() {
        let cfg = Config::parse_from_args(vec!["truswap", "--testnet", "--mainnet"]);
        assert!(!cfg.is_testnet);
        assert_eq!(cfg.chain_id().as_u64(), 8453);
        assert_eq!(cfg.base_usdc_contract, BASE_MAINNET_USDC);
    }

    #[test]
    fn test_custom_htlc_and_usdc_flags() {
        let cfg = Config::parse_from_args(vec![
            "truswap",
            "--base-htlc", "0x1111111111111111111111111111111111111111",
            "--base-usdc", "0x2222222222222222222222222222222222222222",
        ]);
        assert_eq!(cfg.base_htlc_contract, "0x1111111111111111111111111111111111111111");
        assert_eq!(cfg.base_usdc_contract, "0x2222222222222222222222222222222222222222");
    }

    #[test]
    fn test_resync_flag() {
        let cfg = Config::parse_from_args(vec!["truswap", "--resync"]);
        assert!(cfg.resync_from_genesis);
    }

    #[test]
    fn test_relay_flag() {
        let cfg = Config::parse_from_args(vec!["truswap", "--relay"]);
        assert!(cfg.is_relay);
    }

    #[test]
    fn test_fallback_rpcs_selection() {
        let mut cfg = Config::default();
        assert_eq!(cfg.fallback_rpcs(), BASE_MAINNET_FALLBACK_RPCS);
        cfg.set_testnet(true);
        assert_eq!(cfg.fallback_rpcs(), BASE_SEPOLIA_FALLBACK_RPCS);
    }

    #[test]
    fn test_default_relay_node() {
        let cfg = Config::parse_from_args(vec!["truswap"]);
        assert_eq!(cfg.relay_node_url, DEFAULT_RELAY_NODE_URL);
    }

    #[test]
    fn test_custom_relay_node_flag() {
        let cfg = Config::parse_from_args(vec!["truswap", "--relay-node", "127.0.0.1:9090"]);
        assert_eq!(cfg.relay_node_url, "ws://127.0.0.1:9090/orderbook");

        let cfg2 = Config::parse_from_args(vec!["truswap", "--relay-url", "wss://custom.relay.io/mesh"]);
        assert_eq!(cfg2.relay_node_url, "wss://custom.relay.io/mesh");
    }
}
