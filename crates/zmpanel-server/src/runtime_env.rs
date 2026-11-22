use zmpanel_core::RuntimeEnvInfo;

/// Linux `CAP_SYS_ADMIN` bit index in `/proc/self/status` `CapEff`.
const CAP_SYS_ADMIN_BIT: u32 = 21;

#[derive(Debug, Default, Clone)]
pub struct EnvProbeInputs {
    pub is_windows: bool,
    pub has_dockerenv: bool,
    pub has_containerenv: bool,
    pub has_docker_sock: bool,
    pub has_lxd_sock: bool,
    pub has_openvz_markers: bool,
    pub cgroup_text: String,
    pub pid1_environ: String,
    pub proc_version: String,
    pub dmi_text: String,
    pub cap_eff_mask: Option<u64>,
    pub docker_daemon_reachable: bool,
}

/// Pure classification function so every container/virtualization environment can be
/// unit-tested deterministically without mocking `/proc`.
pub fn classify_runtime_env(p: &EnvProbeInputs) -> RuntimeEnvInfo {
    if p.is_windows {
        return RuntimeEnvInfo {
            virt_type: "windows".to_string(),
            virt_label: "Windows 开发/桌面环境".to_string(),
            inside_container: false,
            docker_nesting_supported: true,
            docker_mode: "native".to_string(),
            docker_nesting_reason: "Windows 环境可通过 Docker Desktop / WSL2 运行 Docker"
                .to_string(),
        };
    }

    let cgroup = p.cgroup_text.to_lowercase();
    let env1 = p.pid1_environ.to_lowercase();
    let ver = p.proc_version.to_lowercase();
    let dmi = p.dmi_text.to_lowercase();

    let has_cap_sys_admin = p
        .cap_eff_mask
        .map(|mask| (mask & (1u64 << CAP_SYS_ADMIN_BIT)) != 0)
        .unwrap_or(false);

    // 1. OpenVZ container (shared legacy kernel, cannot run Docker)
    if p.has_openvz_markers {
        return RuntimeEnvInfo {
            virt_type: "openvz".to_string(),
            virt_label: "OpenVZ 容器 (NAT 小鸡)".to_string(),
            inside_container: true,
            docker_nesting_supported: false,
            docker_mode: "unsupported".to_string(),
            docker_nesting_reason:
                "检测到当前服务器为 OpenVZ 容器，共享宿主机内核且缺少现代 namespace/overlayfs 支持，无法运行 Docker。建议直接使用网站管理（原生 Rust/Go/Python 进程守护）或原生软件包。"
                    .to_string(),
        };
    }

    // 2. Kubernetes / Podman / Docker (OCI containers, common for NAT "Docker 鸡")
    let is_k8s = cgroup.contains("kubepods") || env1.contains("kubernetes_service_host=");
    let is_podman = p.has_containerenv
        || cgroup.contains("libpod")
        || cgroup.contains("podman")
        || env1.contains("container=podman");
    let is_docker = p.has_dockerenv
        || cgroup.contains("docker")
        || cgroup.contains("containerd")
        || cgroup.contains("crio")
        || env1.contains("container=docker")
        || env1.contains("container=oci");

    if is_k8s || is_podman || is_docker {
        let (virt_type, virt_label) = if is_k8s {
            ("kubernetes", "Kubernetes Pod 容器")
        } else if is_podman {
            ("podman", "Podman 容器 (NAT 容器鸡)")
        } else {
            ("docker", "Docker 容器 (NAT Docker 鸡)")
        };

        if p.docker_daemon_reachable && p.has_docker_sock {
            return RuntimeEnvInfo {
                virt_type: virt_type.to_string(),
                virt_label: virt_label.to_string(),
                inside_container: true,
                docker_nesting_supported: true,
                docker_mode: "dood".to_string(),
                docker_nesting_reason:
                    "当前本身运行在容器内，但宿主机挂载了 /var/run/docker.sock（DooD 模式），可直接复用宿主机 Docker 引擎。"
                        .to_string(),
            };
        }

        if p.docker_daemon_reachable {
            return RuntimeEnvInfo {
                virt_type: virt_type.to_string(),
                virt_label: virt_label.to_string(),
                inside_container: true,
                docker_nesting_supported: true,
                docker_mode: "dind".to_string(),
                docker_nesting_reason:
                    "当前运行在支持嵌套的容器内，Docker-in-Docker (DinD) 引擎已可用。".to_string(),
            };
        }

        if p.has_docker_sock {
            return RuntimeEnvInfo {
                virt_type: virt_type.to_string(),
                virt_label: virt_label.to_string(),
                inside_container: true,
                docker_nesting_supported: true,
                docker_mode: "dood".to_string(),
                docker_nesting_reason:
                    "检测到容器内已挂载 /var/run/docker.sock（DooD 模式），安装 docker CLI 并确保有 socket 权限后即可管理宿主机容器。"
                        .to_string(),
            };
        }

        if !has_cap_sys_admin {
            return RuntimeEnvInfo {
                virt_type: virt_type.to_string(),
                virt_label: virt_label.to_string(),
                inside_container: true,
                docker_nesting_supported: false,
                docker_mode: "unsupported".to_string(),
                docker_nesting_reason:
                    "检测到当前服务器本身是非特权容器（Docker/OCI 鸡），未挂载 docker.sock 且内核未授予 CAP_SYS_ADMIN 权限，禁止再嵌套运行 Docker（无法套娃）。建议直接使用「网站管理」内置进程守护部署 Rust/Go/Python/反代/静态服务，或在「软件商店」采用原生包安装。"
                        .to_string(),
            };
        }

        return RuntimeEnvInfo {
            virt_type: virt_type.to_string(),
            virt_label: virt_label.to_string(),
            inside_container: true,
            docker_nesting_supported: true,
            docker_mode: "dind".to_string(),
            docker_nesting_reason:
                "当前运行在具备 CAP_SYS_ADMIN 特权的容器内，理论上支持嵌套 Docker (DinD)，但尚未安装或启动 dockerd。"
                    .to_string(),
        };
    }

    // 3. LXC / LXD container (common for Proxmox/SolusVM NAT VPS)
    let is_lxc = p.has_lxd_sock
        || env1.contains("container=lxc")
        || cgroup.contains("/lxc/")
        || cgroup.contains("lxc.payload");
    if is_lxc {
        if p.docker_daemon_reachable {
            return RuntimeEnvInfo {
                virt_type: "lxc".to_string(),
                virt_label: "LXC 容器 (已开启嵌套)".to_string(),
                inside_container: true,
                docker_nesting_supported: true,
                docker_mode: "dind".to_string(),
                docker_nesting_reason: "当前为 LXC 容器，且 Docker 引擎已正常运行。".to_string(),
            };
        }
        if !has_cap_sys_admin {
            return RuntimeEnvInfo {
                virt_type: "lxc".to_string(),
                virt_label: "LXC 容器 (NAT LXC 鸡)".to_string(),
                inside_container: true,
                docker_nesting_supported: false,
                docker_mode: "unsupported".to_string(),
                docker_nesting_reason:
                    "检测到当前服务器为受限 LXC 容器（缺少 CAP_SYS_ADMIN 权限），宿主机未开启 nesting 时无法运行 Docker。建议优先使用「网站管理」原生进程守护部署服务。"
                        .to_string(),
            };
        }
        return RuntimeEnvInfo {
            virt_type: "lxc".to_string(),
            virt_label: "LXC 容器".to_string(),
            inside_container: true,
            docker_nesting_supported: true,
            docker_mode: "dind".to_string(),
            docker_nesting_reason:
                "当前为 LXC 容器（具备 CAP_SYS_ADMIN），若宿主机已启用 nesting=1 及 keyctl，安装 Docker 后即可运行。"
                    .to_string(),
        };
    }

    // 4. WSL / WSL2
    if ver.contains("microsoft") || ver.contains("wsl") {
        return RuntimeEnvInfo {
            virt_type: "wsl".to_string(),
            virt_label: "WSL2 Linux 子系统".to_string(),
            inside_container: false,
            docker_nesting_supported: true,
            docker_mode: "native".to_string(),
            docker_nesting_reason: "WSL2 环境支持原生 Docker 或 Docker Desktop 集成。".to_string(),
        };
    }

    // 5. Full VM or Bare-metal Linux
    let (virt_type, virt_label) = if dmi.contains("kvm") || dmi.contains("qemu") {
        ("kvm", "KVM / QEMU 虚拟机")
    } else if dmi.contains("vmware") {
        ("vmware", "VMware 虚拟机")
    } else if dmi.contains("microsoft corporation") || dmi.contains("hyper-v") {
        ("hyperv", "Hyper-V 虚拟机")
    } else if dmi.contains("virtualbox") || dmi.contains("innotek") {
        ("virtualbox", "VirtualBox 虚拟机")
    } else if dmi.contains("xen") {
        ("xen", "Xen 虚拟机")
    } else {
        ("baremetal", "Linux 独立系统 / 虚拟机")
    };

    RuntimeEnvInfo {
        virt_type: virt_type.to_string(),
        virt_label: virt_label.to_string(),
        inside_container: false,
        docker_nesting_supported: true,
        docker_mode: "native".to_string(),
        docker_nesting_reason: "当前为独立内核环境，完整支持运行 Docker 与 Docker Compose。"
            .to_string(),
    }
}

pub fn parse_cap_eff_from_status(status_text: &str) -> Option<u64> {
    for line in status_text.lines() {
        if let Some(rest) = line.strip_prefix("CapEff:") {
            let hex = rest.trim();
            return u64::from_str_radix(hex, 16).ok();
        }
    }
    None
}

/// Probe current host runtime environment (fast, zero external commands, reads `/proc` & `/sys`).
pub fn detect_runtime_env(docker_daemon_reachable: bool) -> RuntimeEnvInfo {
    #[cfg(not(target_os = "linux"))]
    {
        classify_runtime_env(&EnvProbeInputs {
            is_windows: cfg!(windows),
            docker_daemon_reachable,
            ..Default::default()
        })
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs;
        use std::path::Path;

        let has_dockerenv = Path::new("/.dockerenv").exists() || Path::new("/.dockerinit").exists();
        let has_containerenv = Path::new("/run/.containerenv").exists();
        let has_docker_sock = Path::new("/var/run/docker.sock").exists();
        let has_lxd_sock = Path::new("/dev/lxd/sock").exists();

        // Classic OpenVZ guest check: `/proc/vz` exists while host-only `/proc/bc` does not.
        let has_openvz_markers = (Path::new("/proc/vz").exists() && !Path::new("/proc/bc").exists())
            || (Path::new("/proc/user_beancounters").exists() && !Path::new("/proc/bc").exists());

        let mut cgroup_text = String::new();
        if let Ok(c1) = fs::read_to_string("/proc/1/cgroup") {
            cgroup_text.push_str(&c1);
            cgroup_text.push('\n');
        }
        if let Ok(cs) = fs::read_to_string("/proc/self/cgroup") {
            cgroup_text.push_str(&cs);
        }

        let pid1_environ = fs::read("/proc/1/environ")
            .map(|bytes| {
                bytes
                    .split(|&b| b == 0)
                    .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();

        let proc_version = fs::read_to_string("/proc/version").unwrap_or_default();

        let mut dmi_text = String::new();
        for p in [
            "/sys/class/dmi/id/sys_vendor",
            "/sys/class/dmi/id/product_name",
            "/sys/class/dmi/id/board_vendor",
        ] {
            if let Ok(txt) = fs::read_to_string(p) {
                dmi_text.push_str(&txt);
                dmi_text.push('\n');
            }
        }

        let cap_eff_mask = fs::read_to_string("/proc/self/status")
            .ok()
            .and_then(|s| parse_cap_eff_from_status(&s));

        classify_runtime_env(&EnvProbeInputs {
            is_windows: false,
            has_dockerenv,
            has_containerenv,
            has_docker_sock,
            has_lxd_sock,
            has_openvz_markers,
            cgroup_text,
            pid1_environ,
            proc_version,
            dmi_text,
            cap_eff_mask,
            docker_daemon_reachable,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_unprivileged_docker_nat_vps_cannot_nest() {
        // Typical non-privileged Docker NAT VPS: has /.dockerenv, default Docker CapEff (no CAP_SYS_ADMIN)
        let cap_default_docker = parse_cap_eff_from_status("Name:\tzmpanel\nCapEff:\t00000000a80425fb\n");
        let env = classify_runtime_env(&EnvProbeInputs {
            has_dockerenv: true,
            cgroup_text: "0::/docker/abcdef123456\n".to_string(),
            cap_eff_mask: cap_default_docker,
            docker_daemon_reachable: false,
            ..Default::default()
        });
        assert_eq!(env.virt_type, "docker");
        assert!(env.inside_container);
        assert!(!env.docker_nesting_supported);
        assert_eq!(env.docker_mode, "unsupported");
        assert!(env.docker_nesting_reason.contains("套娃"));
    }

    #[test]
    fn detects_docker_with_mounted_sock_as_dood() {
        let env = classify_runtime_env(&EnvProbeInputs {
            has_dockerenv: true,
            has_docker_sock: true,
            cap_eff_mask: Some(0x00000000a80425fb),
            docker_daemon_reachable: true,
            ..Default::default()
        });
        assert_eq!(env.virt_type, "docker");
        assert!(env.inside_container);
        assert!(env.docker_nesting_supported);
        assert_eq!(env.docker_mode, "dood");
    }

    #[test]
    fn detects_privileged_docker_dind_capable() {
        // Privileged container includes CAP_SYS_ADMIN (bit 21 = 0x200000)
        let env = classify_runtime_env(&EnvProbeInputs {
            has_dockerenv: true,
            cap_eff_mask: Some(0x000001ffffffffff),
            docker_daemon_reachable: false,
            ..Default::default()
        });
        assert_eq!(env.virt_type, "docker");
        assert!(env.inside_container);
        assert!(env.docker_nesting_supported);
        assert_eq!(env.docker_mode, "dind");
    }

    #[test]
    fn detects_openvz_and_restricted_lxc() {
        let ovz = classify_runtime_env(&EnvProbeInputs {
            has_openvz_markers: true,
            ..Default::default()
        });
        assert_eq!(ovz.virt_type, "openvz");
        assert!(ovz.inside_container);
        assert!(!ovz.docker_nesting_supported);

        let lxc = classify_runtime_env(&EnvProbeInputs {
            pid1_environ: "container=lxc\nPATH=/usr/bin".to_string(),
            cap_eff_mask: Some(0x0000000000000000),
            ..Default::default()
        });
        assert_eq!(lxc.virt_type, "lxc");
        assert!(lxc.inside_container);
        assert!(!lxc.docker_nesting_supported);
    }

    #[test]
    fn detects_wsl_and_kvm_as_native_docker_capable() {
        let wsl = classify_runtime_env(&EnvProbeInputs {
            proc_version: "Linux version 6.6.114-microsoft-standard-WSL2".to_string(),
            ..Default::default()
        });
        assert_eq!(wsl.virt_type, "wsl");
        assert!(!wsl.inside_container);
        assert!(wsl.docker_nesting_supported);
        assert_eq!(wsl.docker_mode, "native");

        let kvm = classify_runtime_env(&EnvProbeInputs {
            dmi_text: "QEMU\nStandard PC (Q35 + ICH9, 2009)".to_string(),
            ..Default::default()
        });
        assert_eq!(kvm.virt_type, "kvm");
        assert!(!kvm.inside_container);
        assert!(kvm.docker_nesting_supported);
    }
}
