//! `host-system` 宿主服务实现：会话/设备/服务端事实 + 插件私有文件沙箱。
//!
//! 从 `instance.rs` 拆出（600 行红线，S3s）：每个函数**先过权限门**（`fs` 未声明
//! 即拒 / stat 返回 none），再委托 [`HostContext`](super::HostContext) 的
//! `facts` / `fs`——本文件不含任何业务语义，与宿主平台"只提供通用能力"的边界一致。

use super::{denied, xiaozhi, StoreData};
impl xiaozhi::plugin::host_system::Host for StoreData {
    fn list_sessions(&mut self) -> Vec<xiaozhi::plugin::host_system::SessionInfo> {
        self.ctx
            .facts
            .sessions()
            .into_iter()
            .map(|s| xiaozhi::plugin::host_system::SessionInfo {
                id: s.id,
                device_id: s.device_id,
                phase: s.phase,
                started_at_ms: s.started_at_ms,
            })
            .collect()
    }

    fn get_session(
        &mut self,
        session_id: String,
    ) -> Option<xiaozhi::plugin::host_system::SessionInfo> {
        self.ctx
            .facts
            .session(&session_id)
            .map(|s| xiaozhi::plugin::host_system::SessionInfo {
                id: s.id,
                device_id: s.device_id,
                phase: s.phase,
                started_at_ms: s.started_at_ms,
            })
    }

    fn list_devices(&mut self) -> Vec<xiaozhi::plugin::host_system::DeviceInfo> {
        self.ctx
            .facts
            .devices()
            .into_iter()
            .map(|d| xiaozhi::plugin::host_system::DeviceInfo {
                id: d.id,
                online: d.online,
                last_seen_ms: d.last_seen_ms,
                client_id: d.client_id,
            })
            .collect()
    }

    fn get_device(
        &mut self,
        device_id: String,
    ) -> Option<xiaozhi::plugin::host_system::DeviceInfo> {
        self.ctx
            .facts
            .device(&device_id)
            .map(|d| xiaozhi::plugin::host_system::DeviceInfo {
                id: d.id,
                online: d.online,
                last_seen_ms: d.last_seen_ms,
                client_id: d.client_id,
            })
    }

    fn server_info(&mut self) -> xiaozhi::plugin::host_system::ServerFacts {
        let f = self.ctx.facts.server();
        xiaozhi::plugin::host_system::ServerFacts {
            version: f.version,
            data_dir: f.data_dir,
            started_at_ms: f.started_at_ms,
        }
    }

    fn data_dir(&mut self) -> String {
        self.ctx.facts.server().data_dir
    }

    fn fs_read(&mut self, path: String) -> std::result::Result<Vec<u8>, String> {
        if !self.permissions.fs {
            return Err(denied(&self.plugin_id, "fs-read", "fs"));
        }
        self.ctx
            .fs
            .read(&self.plugin_id, &path)
            .map_err(|e| format!("{e:#}"))
    }

    fn fs_write(&mut self, path: String, bytes: Vec<u8>) -> std::result::Result<(), String> {
        if !self.permissions.fs {
            return Err(denied(&self.plugin_id, "fs-write", "fs"));
        }
        self.ctx
            .fs
            .write(&self.plugin_id, &path, &bytes)
            .map_err(|e| format!("{e:#}"))
    }

    fn fs_list(&mut self, dir: String) -> std::result::Result<Vec<String>, String> {
        if !self.permissions.fs {
            return Err(denied(&self.plugin_id, "fs-list", "fs"));
        }
        self.ctx
            .fs
            .list(&self.plugin_id, &dir)
            .map_err(|e| format!("{e:#}"))
    }

    fn fs_delete(&mut self, path: String) -> std::result::Result<bool, String> {
        if !self.permissions.fs {
            return Err(denied(&self.plugin_id, "fs-delete", "fs"));
        }
        self.ctx
            .fs
            .delete(&self.plugin_id, &path)
            .map_err(|e| format!("{e:#}"))
    }

    fn fs_stat(&mut self, path: String) -> Option<u64> {
        if !self.permissions.fs {
            return None;
        }
        self.ctx.fs.stat(&self.plugin_id, &path)
    }
}

