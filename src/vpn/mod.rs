use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use std::{
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UdpSocket,
    sync::Mutex,
};
use tun::Configuration;



#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct VpnSettings {
    pub enabled: bool,
    pub provider_name: String,
    pub interface_name: String,
    pub wg_config: String,
}

#[derive(Debug, Serialize)]
pub struct VpnStatusResponse {
    pub enabled: bool,
    pub provider_name: String,
    pub interface: String,
    pub connected: bool,
    pub kill_switch_active: bool,
}

#[derive(Debug, Clone)]
pub struct ParsedWgConfig {
    pub private_key: [u8; 32],
    pub interface_ip: Ipv4Addr,
    pub peer_public_key: [u8; 32],
    pub endpoint: SocketAddr,
    pub keepalive: Option<u16>,
}

/// Parses a standard WireGuard `.conf` string (Mullvad, Proton, NordVPN, IVPN, etc.)
pub fn parse_wg_config(raw: &str) -> anyhow::Result<ParsedWgConfig> {
    let mut private_key: Option<[u8; 32]> = None;
    let mut interface_ip: Option<Ipv4Addr> = None;
    let mut peer_public_key: Option<[u8; 32]> = None;
    let mut endpoint: Option<SocketAddr> = None;
    let mut keepalive: Option<u16> = Some(25);

    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('[') {
            continue;
        }

        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim().to_lowercase();
        let val = v.trim();

        match key.as_str() {
            "privatekey" => {
                let bytes = BASE64.decode(val)?;
                let arr: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Invalid PrivateKey length"))?;
                private_key = Some(arr);
            }
            "address" => {
                // Take the first IPv4 CIDR (e.g. "10.64.0.2/32" -> "10.64.0.2")
                for part in val.split(',') {
                    let ip_str = part.trim().split('/').next().unwrap_or("");
                    if let Ok(ip) = ip_str.parse::<Ipv4Addr>() {
                        interface_ip = Some(ip);
                        break;
                    }
                }
            }
            "publickey" => {
                let bytes = BASE64.decode(val)?;
                let arr: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("Invalid PublicKey length"))?;
                peer_public_key = Some(arr);
            }
            "endpoint" => {
                let addr = val
                    .to_socket_addrs()?
                    .find(|a| a.is_ipv4())
                    .ok_or_else(|| anyhow::anyhow!("Could not resolve Endpoint '{}'", val))?;
                endpoint = Some(addr);
            }
            "persistentkeepalive" => {
                keepalive = val.parse::<u16>().ok();
            }
            _ => {}
        }
    }

    Ok(ParsedWgConfig {
        private_key: private_key.ok_or_else(|| anyhow::anyhow!("Missing PrivateKey in config"))?,
        interface_ip: interface_ip.ok_or_else(|| anyhow::anyhow!("Missing IPv4 Address in config"))?,
        peer_public_key: peer_public_key
            .ok_or_else(|| anyhow::anyhow!("Missing Peer PublicKey in config"))?,
        endpoint: endpoint.ok_or_else(|| anyhow::anyhow!("Missing Peer Endpoint in config"))?,
        keepalive,
    })
}

/// Manages the background pure-Rust BoringTun userspace WireGuard tunnel
#[derive(Clone)]
pub struct UserspaceVpnManager {
    connected: Arc<AtomicBool>,
    stop_signal: Arc<AtomicBool>,
    task_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl UserspaceVpnManager {
    pub fn new() -> Self {
        Self {
            connected: Arc::new(AtomicBool::new(false)),
            stop_signal: Arc::new(AtomicBool::new(false)),
            task_handle: Arc::new(Mutex::new(None)),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub async fn stop(&self) {
        self.stop_signal.store(true, Ordering::SeqCst);
        self.connected.store(false, Ordering::SeqCst);
        if let Some(handle) = self.task_handle.lock().await.take() {
            handle.abort();
        }
    }

    pub async fn start(&self, interface_name: &str, raw_conf: &str) -> anyhow::Result<()> {
        self.stop().await;

        let cfg = parse_wg_config(raw_conf)?;
        let secret = StaticSecret::from(cfg.private_key);
        let peer_pub = PublicKey::from(cfg.peer_public_key);

        // 1. Initialize BoringTun cryptographic tunnel
        let mut tunn = Tunn::new(secret, peer_pub, None, cfg.keepalive, 0, None)
            .map_err(|e| anyhow::anyhow!("BoringTun init error: {:?}", e))?;

        // 2. Create userspace TUN device (no system routes added -> automatic split-tunnel!)
        let mut tun_cfg = Configuration::default();
        tun_cfg
            .tun_name(interface_name)
            .address(cfg.interface_ip)
            .netmask((255, 255, 255, 255))
            .mtu(1420)
            .up();

        let mut dev = tun::create_as_async(&tun_cfg)
            .map_err(|e| anyhow::anyhow!("Failed to create TUN device '{}': {} (Hint: run with sudo or CAP_NET_ADMIN)", interface_name, e))?;

        // 3. Bind UDP socket to send/receive encrypted WireGuard packets to the VPN server
        let udp = UdpSocket::bind("0.0.0.0:0").await?;
        udp.connect(cfg.endpoint).await?;

        self.stop_signal.store(false, Ordering::SeqCst);
        self.connected.store(true, Ordering::SeqCst);

        let connected_flag = self.connected.clone();
        let stop_flag = self.stop_signal.clone();
        let iface = interface_name.to_string();

        println!(
            "[VPN] BoringTun userspace tunnel '{}' active -> {} (tunnel IP {})",
            iface, cfg.endpoint, cfg.interface_ip
        );

        // 4. Background packet forwarding loop (TUN <-> BoringTun <-> UDP)
        let handle = tokio::spawn(async move {
            let mut tun_buf = [0u8; 2048];
            let mut udp_buf = [0u8; 2048];
            let mut out_buf = [0u8; 2048];
            let mut timer = tokio::time::interval(Duration::from_millis(250));

            loop {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }

                tokio::select! {
                    // Outbound IP packet from `librqbit` -> encrypt & send over UDP
                    Ok(n) = dev.read(&mut tun_buf) => {
                        match tunn.encapsulate(&tun_buf[..n], &mut out_buf) {
                            TunnResult::WriteToNetwork(packet) => {
                                let _ = udp.send(packet).await;
                            }
                            TunnResult::Err(e) => eprintln!("[VPN] Encapsulate error: {:?}", e),
                            _ => {}
                        }
                    }

                    // Inbound encrypted UDP packet from VPN server -> decrypt & write to TUN
                    Ok(n) = udp.recv(&mut udp_buf) => {
                        match tunn.decapsulate(None, &udp_buf[..n], &mut out_buf) {
                            TunnResult::WriteToTunnelV4(packet, _) | TunnResult::WriteToTunnelV6(packet, _) => {
                                let _ = dev.write_all(packet).await;
                            }
                            TunnResult::WriteToNetwork(packet) => {
                                let _ = udp.send(packet).await;
                                // Flush any queued handshake packets
                                while let TunnResult::WriteToNetwork(p) = tunn.decapsulate(None, &[], &mut out_buf) {
                                    let _ = udp.send(p).await;
                                }
                            }
                            TunnResult::Err(e) => eprintln!("[VPN] Decapsulate error: {:?}", e),
                            _ => {}
                        }
                    }

                    // Periodic WireGuard handshake & keepalive timer
                     _ = timer.tick() => {
                        match tunn.update_timers(&mut out_buf) {
                            TunnResult::WriteToNetwork(packet) => {
                                let _ = udp.send(packet).await;
                            }
                            TunnResult::Err(boringtun::noise::errors::WireGuardError::ConnectionExpired) => {
                                // Normal 180s WireGuard session expiry: trigger a clean re-handshake silently
                                if let TunnResult::WriteToNetwork(hs) = tunn.format_handshake_initiation(&mut out_buf, false) {
                                    let _ = udp.send(hs).await;
                                }
                            }
                            TunnResult::Err(e) => {
                                eprintln!("[VPN] Tunnel timer error: {:?}", e);
                                connected_flag.store(false, Ordering::SeqCst);
                            }
                            _ => {}
                        }
                    }
                }
            }

            connected_flag.store(false, Ordering::SeqCst);
            println!("[VPN] Userspace tunnel '{}' stopped.", iface);
        });

        *self.task_handle.lock().await = Some(handle);
        Ok(())
    }
}

pub async fn load_vpn_settings(db: &SqlitePool) -> anyhow::Result<VpnSettings> {
    let settings = sqlx::query_as::<_, VpnSettings>(
        "SELECT enabled, provider_name, interface_name, wg_config FROM vpn_settings WHERE id = 1",
    )
    .fetch_one(db)
    .await?;
    Ok(settings)
}
