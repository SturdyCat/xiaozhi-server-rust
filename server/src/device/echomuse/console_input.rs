use super::*;

impl ConsoleSession {
    /// 处理一条浏览器文本帧。被拒（限流/超长）时**一个字节都不写设备**：
    /// 决策由 `frames::plan_line`/`plan_ctrl_c` 给出，`Reject` 不携带字节。
    pub(super) async fn on_browser_text(
        &mut self,
        text: &str,
        browser: &mut WebSocket,
        device: &mut WebSocket,
    ) {
        match frames::parse_incoming(text) {
            frames::Incoming::Ping => self.send(browser, &ConsoleFrame::Pong).await,
            frames::Incoming::CtrlC => {
                match frames::plan_ctrl_c(&mut self.rate, Instant::now(), self.pty) {
                    frames::InputAction::Write(bytes) => {
                        self.write_device(device, bytes, "^C").await;
                    }
                    frames::InputAction::Reject(code) => self.reject(browser, code).await,
                }
            }
            frames::Incoming::Input(data) => {
                match frames::plan_line(&mut self.rate, Instant::now(), &data, self.pty) {
                    frames::InputAction::Write(bytes) => {
                        self.write_device(device, bytes, &data).await;
                    }
                    frames::InputAction::Reject(code) => self.reject(browser, code).await,
                }
            }
            frames::Incoming::RawInput(data) => {
                match frames::plan_raw(&data, &mut self.raw_rate, Instant::now(), self.pty) {
                    frames::InputAction::Write(bytes) => {
                        self.write_device(device, bytes, &data).await
                    }
                    frames::InputAction::Reject(code) => self.reject(browser, code).await,
                }
            }
            frames::Incoming::Resize(cols, rows) => {
                match frames::plan_resize(cols, rows, &mut self.raw_rate, Instant::now(), self.pty)
                {
                    frames::InputAction::Write(bytes) => {
                        let _ = device.send(Message::Binary(bytes.into())).await;
                    }
                    frames::InputAction::Reject(code) => self.reject(browser, code).await,
                }
            }
            frames::Incoming::InvalidResize => self.reject(browser, ErrorCode::InvalidResize).await,
            frames::Incoming::Unknown => {}
        }
    }
}
