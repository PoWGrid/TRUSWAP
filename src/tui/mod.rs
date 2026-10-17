use std::collections::VecDeque;
use std::io::{stdout, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

static CLEANUP_DONE: AtomicBool = AtomicBool::new(false);

pub struct TuiState {
    pub start_time: Instant,
    pub ui_url: String,
    pub tru_block: u32,
    pub tru_balance_atoms: u64,
    pub tru_vault_addr: String,
    pub tru_p2p_peers: u64,
    pub base_contract: String,
    pub network_name: String,
    pub base_orders_count: usize,
    pub tru_orders_count: usize,
    pub ws_clients_count: usize,
    pub last_event: String,
    pub events: VecDeque<String>,
}

#[derive(Clone)]
pub struct TuiManager {
    is_tty: bool,
    state: Arc<Mutex<TuiState>>,
    ui_events: Arc<tokio::sync::RwLock<Option<tokio::sync::broadcast::Sender<serde_json::Value>>>>,
}

impl TuiManager {
    pub fn new(
        ui_url: String,
        base_contract: String,
        tru_vault_addr: String,
        network_name: String,
    ) -> Self {
        let is_tty = stdout().is_terminal();
        if is_tty {
            // Enter alternate screen buffer, disable wrap, clear screen, cursor home, hide cursor
            print!("\x1b[?1049h\x1b[?7l\x1b[2J\x1b[H\x1b[?25l");
            stdout().flush().ok();
        }

        let state = Arc::new(Mutex::new(TuiState {
            start_time: Instant::now(),
            ui_url,
            tru_block: 25_032,
            tru_balance_atoms: 0,
            tru_vault_addr,
            tru_p2p_peers: 0,
            base_contract,
            network_name,
            base_orders_count: 0,
            tru_orders_count: 0,
            ws_clients_count: 0,
            last_event: "Initializing TRUSWAP runner...".to_string(),
            events: VecDeque::with_capacity(200),
        }));

        Self {
            is_tty,
            state,
            ui_events: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    pub async fn set_ui_events(&self, tx: tokio::sync::broadcast::Sender<serde_json::Value>) {
        let mut w = self.ui_events.write().await;
        *w = Some(tx);
    }

    pub async fn get_events(&self) -> Vec<String> {
        let st = self.state.lock().await;
        st.events.iter().cloned().collect()
    }

    pub async fn log(&self, msg: String) {
        let time_str = chrono::Local::now().format("%H:%M:%S").to_string();
        let entry = format!("[{}] {}", time_str, msg);
        {
            let mut st = self.state.lock().await;
            st.last_event = msg.clone();
            if st.events.len() >= 200 {
                st.events.pop_front();
            }
            st.events.push_back(entry.clone());
        }
        if let Some(ref tx) = *self.ui_events.read().await {
            let _ = tx.send(serde_json::json!({
                "type": "automation_log",
                "data": {
                    "entry": entry,
                    "msg": msg,
                    "time": time_str
                }
            }));
        }
    }

    pub async fn update_tru_state(&self, block: u32, balance_atoms: u64, vault_addr: String, p2p_peers: u64) {
        let mut st = self.state.lock().await;
        st.tru_block = block;
        st.tru_balance_atoms = balance_atoms;
        st.tru_p2p_peers = p2p_peers;
        if !vault_addr.is_empty() {
            st.tru_vault_addr = vault_addr;
        }
    }

    pub async fn update_orders_count(&self, base_count: usize, tru_count: usize) {
        let mut st = self.state.lock().await;
        st.base_orders_count = base_count;
        st.tru_orders_count = tru_count;
    }

    pub async fn set_ws_clients(&self, count: usize) {
        let mut st = self.state.lock().await;
        st.ws_clients_count = count;
    }

    pub async fn render(&self) {
        if !self.is_tty {
            return;
        }

        let st = self.state.lock().await;
        let uptime = st.start_time.elapsed().as_secs();
        let hours = uptime / 3600;
        let mins = (uptime % 3600) / 60;
        let secs = uptime % 60;

        let width = 76;
        let inner_width = 74;

        let mut out = String::with_capacity(4096);
        // Zero-flicker cursor home
        out.push_str("\x1b[H");

        let border_top = format!("╭{}╮\x1b[K\n", "─".repeat(width));
        let border_div = format!("├{}┤\x1b[K\n", "─".repeat(width));
        let border_bot = format!("╰{}╯\x1b[K\n", "─".repeat(width));

        out.push_str(&border_top);
        out.push_str(&box_row(
            "\x1b[1;36m► POWGRID TRUSWAP ($TRU) CROSS-CHAIN P2P ATOMIC RUNNER v1.0.0\x1b[0m",
            inner_width,
        ));

        // UI URL
        out.push_str(&two_col_row(
            &format!("\x1b[90mWeb UI  :\x1b[0m \x1b[1;33m{}\x1b[0m", st.ui_url),
            40,
            &format!("\x1b[90mUptime  :\x1b[0m \x1b[1;37m{:02}:{:02}:{:02}\x1b[0m", hours, mins, secs),
            inner_width,
        ));

        out.push_str(&border_div);
        out.push_str(&box_row("\x1b[1;35mCROSS-CHAIN NETWORKS & P2P DAEMON STATUS\x1b[0m", inner_width));

        // Node RPC & Base contract
        let tru_status = if st.tru_p2p_peers > 0 {
            format!("\x1b[90mTRU (P2P):\x1b[0m \x1b[1;32m● Online ({} peers)\x1b[0m \x1b[90m(#{})", st.tru_p2p_peers, st.tru_block)
        } else {
            format!("\x1b[90mTRU Node:\x1b[0m \x1b[1;32m● CONNECTED\x1b[0m \x1b[90m(#{})", st.tru_block)
        };

        out.push_str(&two_col_row(
            &tru_status,
            42,
            &format!("\x1b[90m{}:\x1b[0m \x1b[1;32m● WATCHING\x1b[0m", st.network_name),
            inner_width,
        ));

        let short_base = if st.base_contract.len() >= 14 {
            format!("{}...{}", &st.base_contract[..8], &st.base_contract[st.base_contract.len() - 6..])
        } else {
            st.base_contract.clone()
        };

        out.push_str(&two_col_row(
            &format!("\x1b[90mTRU Mode:\x1b[0m \x1b[1;36mP2P SPV\x1b[0m"),
            42,
            &format!("\x1b[90mBaseHTLC    :\x1b[0m \x1b[36m{}\x1b[0m", short_base),
            inner_width,
        ));

        out.push_str(&two_col_row(
            &format!("\x1b[90mProtocol:\x1b[0m \x1b[1;36mHTLC Script\x1b[0m"),
            40,
            &format!("\x1b[90mWebSocket   :\x1b[0m \x1b[1;32m● {} Active\x1b[0m", st.ws_clients_count),
            inner_width,
        ));

        out.push_str(&border_div);
        out.push_str(&box_row("\x1b[1;33mTRU VAULT & ORDERBOOK TELEMETRY\x1b[0m", inner_width));

        let tru_coins = st.tru_balance_atoms as f64 / 100_000_000.0;
        let short_vault = if st.tru_vault_addr.len() >= 16 {
            format!("{}...{}", &st.tru_vault_addr[..10], &st.tru_vault_addr[st.tru_vault_addr.len() - 4..])
        } else {
            st.tru_vault_addr.clone()
        };

        out.push_str(&two_col_row(
            &format!("\x1b[90mVault Bal:\x1b[0m \x1b[1;33m{:.8} TRU\x1b[0m", tru_coins),
            40,
            &format!("\x1b[90mVault Addr  :\x1b[0m \x1b[36m{}\x1b[0m", short_vault),
            inner_width,
        ));

        out.push_str(&two_col_row(
            &format!("\x1b[90mBase Bids:\x1b[0m \x1b[1;32m{} Active Orders\x1b[0m", st.base_orders_count),
            40,
            &format!("\x1b[90mTRU Asks    :\x1b[0m \x1b[1;37m{} Active Orders\x1b[0m", st.tru_orders_count),
            inner_width,
        ));

        out.push_str(&two_col_row(
            &format!("\x1b[90mHTLC Rate:\x1b[0m \x1b[1;37mDynamic (P2P Market)\x1b[0m"),
            40,
            &format!("\x1b[90mSettlement  :\x1b[0m \x1b[1;32mTrustless P2P Atomic\x1b[0m"),
            inner_width,
        ));

        out.push_str(&border_div);
        out.push_str(&box_row("\x1b[1;36mLIVE EVENT LOG (Latest Activity)\x1b[0m", inner_width));

        let ev_vec: Vec<&String> = st.events.iter().rev().take(6).collect();
        for i in 0..6 {
            if let Some(ev) = ev_vec.get(i) {
                out.push_str(&box_row(&truncate_visible(ev, inner_width), inner_width));
            } else {
                out.push_str(&box_row("\x1b[90m...\x1b[0m", inner_width));
            }
        }

        out.push_str(&border_bot);
        out.push_str("  \x1b[90m[Open http://127.0.0.1:8899 in browser to trade | Press CTRL+C to exit]\x1b[0m\x1b[K\n");
        out.push_str("\x1b[J");

        print!("{}", out);
        stdout().flush().ok();
    }

    pub fn restore_terminal() {
        if !CLEANUP_DONE.swap(true, Ordering::SeqCst) {
            print!("\x1b[?7h\x1b[?25h\x1b[?1049l\n");
            stdout().flush().ok();
        }
    }
}

impl Drop for TuiManager {
    fn drop(&mut self) {
        Self::restore_terminal();
    }
}

fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_esc = true;
        } else if in_esc {
            if c == 'm' {
                in_esc = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

fn truncate_visible(s: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut current_width = 0;
    let mut in_esc = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_esc = true;
            out.push(c);
        } else if in_esc {
            out.push(c);
            if c == 'm' {
                in_esc = false;
            }
        } else {
            if current_width + 1 <= max_width {
                out.push(c);
                current_width += 1;
            } else {
                break;
            }
        }
    }
    out.push_str("\x1b[0m");
    out
}

fn box_row(content: &str, inner_width: usize) -> String {
    let vlen = visible_len(content);
    let padding = if vlen < inner_width { inner_width - vlen } else { 0 };
    format!("│ {}{} │\x1b[K\n", content, " ".repeat(padding))
}

fn two_col_row(left: &str, left_col_width: usize, right: &str, inner_width: usize) -> String {
    let l_vlen = visible_len(left);
    let l_pad = if l_vlen < left_col_width { left_col_width - l_vlen } else { 2 };
    let r_vlen = visible_len(right);
    let total_used = l_vlen + l_pad + r_vlen;
    let r_pad = if total_used < inner_width { inner_width - total_used } else { 0 };
    format!("│ {}{}{}{} │\x1b[K\n", left, " ".repeat(l_pad), right, " ".repeat(r_pad))
}
