//! mDNS 广播服务：插件声明自己要广播的服务，宿主代发。
//!
//! 与 `_xiaozhi-admin._tcp`（管理面发现）互不干扰：这里只有**通用**能力，
//! 服务类型、实例名、TXT 全由插件给，宿主不解释语义。
//!
//! 生命周期：句柄随插件 `deactivate` 一并撤销（[`MdnsAdvertiser::clear_plugin`]）。

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};

/// 已注册的广播。
struct Advert {
    plugin_id: String,
    fullname: String,
}

/// 主机名（A 记录归属）：与 mDNS 惯例一致，冲突由 daemon 自行改名协商。
const HOST_NAME: &str = "xiaozhi.local.";

/// mDNS 广播器（进程内单例，`ServiceDaemon` 只起一次）。
pub struct MdnsAdvertiser {
    daemon: Option<mdns_sd::ServiceDaemon>,
    adverts: Mutex<BTreeMap<u64, Advert>>,
    next_id: AtomicU64,
    /// 广播地址：默认出口 IPv4（探测失败退回 loopback，跨机不可见但不 panic）。
    ip: Ipv4Addr,
}

impl MdnsAdvertiser {
    /// 创建广播器；daemon 起不来时降级为「只记录不广播」并返回 Err 供调用方告警。
    pub fn new() -> (Arc<Self>, Result<()>) {
        let ip = local_ipv4().unwrap_or(Ipv4Addr::LOCALHOST);
        match mdns_sd::ServiceDaemon::new() {
            Ok(daemon) => (
                Arc::new(Self {
                    daemon: Some(daemon),
                    adverts: Mutex::new(BTreeMap::new()),
                    next_id: AtomicU64::new(1),
                    ip,
                }),
                Ok(()),
            ),
            Err(e) => (
                Arc::new(Self {
                    daemon: None,
                    adverts: Mutex::new(BTreeMap::new()),
                    next_id: AtomicU64::new(1),
                    ip,
                }),
                Err(anyhow::anyhow!("mDNS daemon 启动失败：{e}")),
            ),
        }
    }

    /// 广播一个服务，返回句柄。
    pub fn advertise(
        &self,
        plugin_id: &str,
        service_type: &str,
        instance: &str,
        port: u16,
        txt: &[(String, String)],
    ) -> Result<u64> {
        let Some(daemon) = &self.daemon else {
            bail!("mDNS 不可用（daemon 未启动）");
        };
        let props = if txt.is_empty() {
            None
        } else {
            Some(txt.iter().cloned().collect())
        };
        let info = mdns_sd::ServiceInfo::new(
            service_type,
            instance,
            HOST_NAME,
            self.ip.to_string(),
            port,
            props,
        )
        .map_err(|e| anyhow::anyhow!("mDNS 服务描述非法（{service_type}/{instance}）：{e}"))?;

        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .map_err(|e| anyhow::anyhow!("mDNS 广播失败（{fullname}）：{e}"))?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.adverts
            .lock()
            .map_err(|_| anyhow::anyhow!("mDNS 句柄表锁中毒"))?
            .insert(
                id,
                Advert {
                    plugin_id: plugin_id.to_string(),
                    fullname,
                },
            );
        Ok(id)
    }

    /// 撤销广播。
    pub fn unadvertise(&self, plugin_id: &str, handle: u64) -> Result<()> {
        let advert = {
            let mut adverts = self
                .adverts
                .lock()
                .map_err(|_| anyhow::anyhow!("mDNS 句柄表锁中毒"))?;
            match adverts.get(&handle) {
                Some(a) if a.plugin_id == plugin_id => adverts.remove(&handle),
                Some(a) => bail!("mDNS 句柄 {handle} 属于插件 {}，非 {plugin_id}", a.plugin_id),
                None => bail!("mDNS 句柄 {handle} 不存在"),
            }
        };
        if let (Some(daemon), Some(advert)) = (&self.daemon, advert) {
            let _ = daemon.unregister(&advert.fullname);
        }
        Ok(())
    }

    /// 撤销某插件的全部广播（deactivate / remove）。
    pub fn clear_plugin(&self, plugin_id: &str) {
        let removed: Vec<Advert> = {
            let Ok(mut adverts) = self.adverts.lock() else {
                return;
            };
            let ids: Vec<u64> = adverts
                .iter()
                .filter(|(_, a)| a.plugin_id == plugin_id)
                .map(|(id, _)| *id)
                .collect();
            ids.into_iter()
                .filter_map(|id| adverts.remove(&id))
                .collect()
        };
        if let Some(daemon) = &self.daemon {
            for advert in removed {
                let _ = daemon.unregister(&advert.fullname);
            }
        }
    }

    pub fn count(&self) -> usize {
        self.adverts.lock().map(|a| a.len()).unwrap_or(0)
    }
}

/// 默认出口局域网 IPv4：UDP `connect` 只用内核路由表选源地址，不发包。
///
/// 刻意不复用 `device::local_ipv4`——插件平台不能依赖任何设备业务模块。
fn local_ipv4() -> Option<Ipv4Addr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) if !v4.ip().is_loopback() => Some(*v4.ip()),
        _ => None,
    }
}
