use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const MAX_WS_FRAME_BYTES: usize = 4096;
pub const MAX_CONCURRENT_PER_IP: usize = 256;
pub const SUBMISSION_COOLDOWN_MILLIS: u128 = 500;

/// Tunes Linux kernel file descriptor limits up to 1,048,576 for ultrablazing performance
#[cfg(target_os = "linux")]
pub fn tune_kernel_limits() {
    unsafe {
        let rlimit = libc::rlimit {
            rlim_cur: 1_048_576,
            rlim_max: 1_048_576,
        };
        if libc::setrlimit(libc::RLIMIT_NOFILE, &rlimit) != 0 {
            tracing::warn!("[RELAY-PROTECT] Could not set RLIMIT_NOFILE to 1M, running on default OS limits");
        } else {
            tracing::info!("[RELAY-PROTECT] System file descriptor limit successfully elevated to 1,048,576");
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn tune_kernel_limits() {
    // Non-Linux platforms do not require setrlimit
}

pub struct ConnectionGuard {
    pub max_clients: usize,
    pub active_clients: AtomicUsize,
    ip_connections: Mutex<HashMap<IpAddr, usize>>,
    submission_timestamps: Mutex<HashMap<String, Instant>>,
}

impl ConnectionGuard {
    pub fn new(max_clients: usize) -> Arc<Self> {
        Arc::new(Self {
            max_clients,
            active_clients: AtomicUsize::new(0),
            ip_connections: Mutex::new(HashMap::new()),
            submission_timestamps: Mutex::new(HashMap::new()),
        })
    }

    /// Attempts to register a new incoming connection.
    /// Returns Ok(()) if within limits, or Err with reason if quota exceeded.
    pub fn acquire_connection(&self, ip: IpAddr) -> Result<(), &'static str> {
        let current_total = self.active_clients.load(Ordering::Relaxed);
        if current_total >= self.max_clients {
            return Err("Relay connection capacity reached (503 Service Unavailable)");
        }

        let mut ips = self.ip_connections.lock().unwrap();
        let ip_count = ips.entry(ip).or_insert(0);
        if *ip_count >= MAX_CONCURRENT_PER_IP {
            return Err("Too many active connections from this IP address");
        }

        *ip_count += 1;
        self.active_clients.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Releases a closed connection and decrements IP and total counters
    pub fn release_connection(&self, ip: IpAddr) {
        self.active_clients.fetch_sub(1, Ordering::Relaxed);
        let mut ips = self.ip_connections.lock().unwrap();
        if let Some(count) = ips.get_mut(&ip) {
            if *count > 1 {
                *count -= 1;
            } else {
                ips.remove(&ip);
            }
        }
    }

    /// Validates submission rate limit per wallet address (anti-spam cooldown)
    pub fn check_submission_rate(&self, wallet_address: &str) -> Result<(), &'static str> {
        let mut subs = self.submission_timestamps.lock().unwrap();
        let now = Instant::now();
        if let Some(last_time) = subs.get(wallet_address) {
            if now.duration_since(*last_time).as_millis() < SUBMISSION_COOLDOWN_MILLIS {
                return Err("Order submission rate limit exceeded. Please wait a moment.");
            }
        }
        subs.insert(wallet_address.to_string(), now);
        Ok(())
    }
}
