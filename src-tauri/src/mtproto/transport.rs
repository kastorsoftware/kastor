use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::i18n::t_with;
use crate::proxy::{self, ProxyConfig};

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const IO_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);

// mtproto TcpFull transport
// wire format: length(4) + seq_no(4) + payload + crc32(4)

pub struct MtpTransport {
    stream: TcpStream,
    send_seq: u32,
    recv_seq: u32,
}

impl MtpTransport {
    pub async fn connect(addr: &str, proxy: Option<&ProxyConfig>) -> Result<Self, String> {
        dbg_log!("transport::connect addr={} proxy={}", addr, proxy.is_some());

        let stream = if let Some(px) = proxy {
            let (host, port) = parse_host_port(addr)?;
            proxy::connect_via_proxy(px, host, port).await?
        } else {
            tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
                .await
                .map_err(|_| "MTProto connection timed out".to_string())?
                .map_err(|e| t_with("mtproto_connect_error", &[("error", &e.to_string())]))?
        };

        dbg_log!("transport::connect established OK");
        Ok(Self {
            stream,
            send_seq: 0,
            recv_seq: 0,
        })
    }

    pub async fn send(&mut self, data: &[u8]) -> Result<(), String> {
        let packet_len = (data.len() + 12) as u32;
        let seq = self.send_seq;
        self.send_seq += 1;

        let mut packet = Vec::with_capacity(packet_len as usize);
        packet.extend_from_slice(&packet_len.to_le_bytes());
        packet.extend_from_slice(&seq.to_le_bytes());
        packet.extend_from_slice(data);

        let crc = crc32(&packet);
        packet.extend_from_slice(&crc.to_le_bytes());

        tokio::time::timeout(IO_TIMEOUT, self.stream.write_all(&packet))
            .await
            .map_err(|_| "MTProto write timed out".to_string())?
            .map_err(|e| format!("write failed: {e}"))?;
        tokio::time::timeout(IO_TIMEOUT, self.stream.flush())
            .await
            .map_err(|_| "MTProto flush timed out".to_string())?
            .map_err(|e| format!("flush failed: {e}"))?;

        dbg_log!("transport::send {} bytes OK", packet.len());
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Vec<u8>, String> {
        let mut len_buf = [0u8; 4];
        tokio::time::timeout(IO_TIMEOUT, self.stream.read_exact(&mut len_buf))
            .await
            .map_err(|_| "MTProto read timed out".to_string())?
            .map_err(|e| format!("read length failed: {e}"))?;

        let packet_len = u32::from_le_bytes(len_buf) as usize;

        // negative value = transport error
        let len_i32 = i32::from_le_bytes(len_buf);
        if len_i32 < 0 {
            return Err(format!("transport error: {}", len_i32));
        }

        if packet_len < 12 {
            return Err(format!("packet too small: {}", packet_len));
        }
        if packet_len > 16 * 1024 * 1024 {
            return Err("response too large".into());
        }

        let remaining = packet_len - 4;
        let mut rest = vec![0u8; remaining];
        tokio::time::timeout(IO_TIMEOUT, self.stream.read_exact(&mut rest))
            .await
            .map_err(|_| "MTProto read timed out".to_string())?
            .map_err(|e| format!("read data failed: {e}"))?;

        // verify crc32
        let crc_offset = remaining - 4;
        let received_crc = u32::from_le_bytes([
            rest[crc_offset],
            rest[crc_offset + 1],
            rest[crc_offset + 2],
            rest[crc_offset + 3],
        ]);

        let mut check_buf = Vec::with_capacity(packet_len - 4);
        check_buf.extend_from_slice(&len_buf);
        check_buf.extend_from_slice(&rest[..crc_offset]);
        let calculated_crc = crc32(&check_buf);

        if received_crc != calculated_crc {
            return Err("crc32 mismatch".into());
        }

        self.recv_seq += 1;

        let payload = rest[4..crc_offset].to_vec();
        dbg_log!("transport::recv {} bytes payload OK", payload.len());
        Ok(payload)
    }
}

fn parse_host_port(addr: &str) -> Result<(&str, u16), String> {
    let colon = addr.rfind(':').ok_or("invalid address format")?;
    let host = &addr[..colon];
    let port = addr[colon + 1..]
        .parse::<u16>()
        .map_err(|_| "invalid port in address".to_string())?;
    Ok((host, port))
}

fn crc32(data: &[u8]) -> u32 {
    static TABLE: [u32; 256] = {
        let mut t = [0u32; 256];
        let mut i = 0u32;
        while i < 256 {
            let mut c = i;
            let mut j = 0;
            while j < 8 {
                if c & 1 != 0 {
                    c = (c >> 1) ^ 0xEDB88320;
                } else {
                    c >>= 1;
                }
                j += 1;
            }
            t[i as usize] = c;
            i += 1;
        }
        t
    };
    let mut crc = 0xFFFFFFFFu32;
    for &b in data {
        crc = TABLE[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFFFFFF
}
