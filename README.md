# zmpanel

面向个人多服务器（含 **256MB 内存 NAT 小鸡 / Docker 鸡 / LXC 鸡**）的极简、安全、无外联、无广告、超低内存现代化 Linux 运维面板。

## 核心特点
- **极致低内存（Linux Release 实测）**：
  - 纯受控 **NAT Agent 模式**：常驻 **`9.5 MB`**（私有堆 **`1.8 MB`**，峰值 **`9.5 MB`**）
  - 完整 **Master / 单机模式**（含内嵌前端、监控、安全引擎、Cron）：登录后常驻 **`10.7 MB`**（私有堆 **`2.4 MB`**）
  - 单二进制体积 **`14 MB`**（`musl` 静态编译，Debian / Ubuntu / Alpine 开箱即用）
- **零外联、零广告、零云端绑定**：完全离线运行，不强制登录官方账号，不上传任何遥测数据。
- **安全优先**：默认随机安全入口（非入口访问返回伪装 404）、Argon2id 密码哈希 + TOTP 2FA、登录失败锁定、严禁拼接 Shell 字符串、完整操作审计日志、防火墙防自锁（自动保护 SSH 与面板端口）。
- **直击传统面板痛点**：
  - **CodeMirror 6** 文件编辑器（无损全选与复制粘贴，告别截断 Bug）
  - **xterm.js + 原生 PTY** Web 终端（完美 `Ctrl+Shift+C/V` 复制粘贴、多标签页、命令收藏夹）
  - **宿主机虚拟化与 Docker 嵌套自动检测**：自动识别 NAT Docker 鸡 / LXC / OpenVZ 无法套娃 Docker、DooD / DinD 并给出原生部署引导
  - **Master / NAT Agent 反向加密通道**：NAT 小鸡无需公网 IP 与端口映射即可接入主控，支持批量下发命令、一键批量更新 Nginx 等软件、小鸡一键备份至主控中央仓库与换新机一键迁移恢复
  - **6 套主题 + 6 种强调色**自由搭配

---

## 一键安装（Debian / Ubuntu / Alpine）

### 1. 单机模式（Standalone）
```bash
curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash
```

### 2. 主控模式（Master，可同时管理本机 + 多台 NAT 小鸡）
```bash
curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- --mode master --port 18080
```

### 3. NAT 受控节点（Agent 模式，免开端口反向连主控）
在主控面板「节点集群 → 新增节点」即可一键生成并复制完整接入命令：
```bash
curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- \
  --mode agent \
  --master-url "wss://你的主控域名/安全入口/api/agent/ws" \
  --node-id "node_xxxxxxxx" \
  --node-token "生成的节点令牌"
```

### 4. 国内服务器加速 / 离线二进制 / 源码编译安装
```bash
# 使用 GitHub 代理前缀加速下载
GH_PROXY="https://ghfast.top/" curl -fsSL https://ghfast.top/https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo -E bash

# 使用本地已编译好的二进制文件安装
sudo bash scripts/install.sh --bin ./zmpanel --mode master --port 18080

# 直接在目标机从源码编译安装（需已装 git / cargo / pnpm）
curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- --from-source
```

---

## 卸载方式（单卸面板 / 完全卸载服务）

安装完成后系统会自动注册快捷管理命令 `zmpanel-ctl`（无需再次联网下载脚本）：

### 方式 A：交互式卸载向导（推荐）
```bash
sudo zmpanel-ctl uninstall
```
支持 3 种模式自由选择：
1. **仅卸载面板本身**：停止并移除面板服务与二进制，**完好保留**网站数据、SSL、Nginx、Docker、数据库与 `/opt/zmpanel/data`；
2. **卸载面板 + 清理面板系统配置**：额外删除面板数据目录、面板创建的 `nftables` 独立表 (`inet zmpanel`) 与 `/swapfile.zmpanel`（仍保留业务软件）；
3. **完全卸载（面板 + 关联服务）**：自动检测本机已安装的 **Nginx、Redis、PostgreSQL、MongoDB、Python `uv`、Docker**，可交互式选择全部或指定几项一并卸载（卸载 Docker 时会单独二次确认是否清空 `/var/lib/docker`）。

### 方式 B：非交互命令行一键卸载
```bash
# 1) 仅卸载面板（保留数据与所有服务）
sudo zmpanel-ctl uninstall --uninstall-panel --yes

# 2) 卸载面板并清理面板数据、防火墙规则表与 Swap
sudo zmpanel-ctl uninstall --purge-panel --yes

# 3) 完全卸载：面板 + 指定关联服务（示例：卸载 Nginx 和 Redis）
sudo zmpanel-ctl uninstall --uninstall-all --remove-apps nginx,redis --yes

# 4) 彻底清空：面板 + 全部检测到的服务 + 清空 /var/lib/docker（谨慎使用！）
sudo zmpanel-ctl uninstall --uninstall-all --purge-docker-data --yes
```

> 若服务器上尚未安装 `zmpanel-ctl`，也可直接在线调用卸载脚本：
> ```bash
> curl -fsSL https://raw.githubusercontent.com/zsancc/zmpanel/main/scripts/install.sh | sudo bash -s -- --uninstall
> ```

### 常用运维快捷命令
```bash
sudo zmpanel-ctl status    # 查看运行状态
sudo zmpanel-ctl info      # 查看启动日志、随机入口与初始密码
sudo zmpanel-ctl restart   # 重启服务
sudo zmpanel-ctl upgrade   # 一键拉取最新 Release 并校验 SHA256 升级
```

---

## 发布 Release（触发 GitHub Actions 自动构建）

仓库已内置 [release.yml](.github/workflows/release.yml)。推送版本标签即可自动构建并发布 `x86_64` 与 `aarch64` Linux `musl` 静态单二进制及面板自更新所需的 `release-manifest-*.json`：

```bash
git tag v0.1.0
git push origin v0.1.0
```

---

## 本地开发与构建

```bash
# 1. 构建前端
cd web && pnpm install && pnpm build

# 2. 运行测试与后端
cd ..
cargo test
cargo run -p zmpanel-server -- --mode master --port 18080 --data-dir ./dev-data
```
