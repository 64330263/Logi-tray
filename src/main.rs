use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::path::PathBuf;
use tray_icon::{
    menu::{Menu, MenuItem},
    TrayIcon, TrayIconBuilder,
    icon::Icon,
};
use hidapi::HidApi;
use hidpp::device::Device;
use windows_registry::CURRENT_USER;
use notify_rust::Notification;

// Logitech Vendor ID 046D
const LOGITECH_VID: u16 = 0x046D;
// G502 Lightspeed PID: c08e
const G502_LS_PID: u16 = 0xc08e;
const REG_RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const REG_APP_NAME: &str = "LogiTrayG502";

// 根据电量获取对应系统图标
fn get_icon_by_battery(pct:u8) -> Icon {
    if pct >= 51 {
        // 充足 - 信息图标(蓝色系)
        Icon::from_resource(104, Some((64,64))).unwrap()
    } else if pct >=21 {
        // 中等 - 警告(黄色)
        Icon::from_resource(101, Some((64,64))).unwrap()
    } else {
        // 低电量 - 错误(红色)
        Icon::from_resource(103, Some((64,64))).unwrap()
    }
}

fn get_g502_battery() -> Option<u8> {
    let api = match HidApi::new() {
        Ok(a) => a,
        Err(_) => return None,
    };
    for dev_info in api.device_list() {
        if dev_info.vendor_id() == LOGITECH_VID && dev_info.product_id() == G502_LS_PID {
            if let Ok(hid_dev) = dev_info.open_device(&api) {
                let mut dev = Device::new(hid_dev);
                if let Ok(batt) = dev.get_battery_status() {
                    return Some(batt.level);
                }
            }
        }
    }
    None
}

// 判断是否已经设置开机自启
fn is_autostart_enabled() -> bool {
    if let Ok(key) = CURRENT_USER.open(REG_RUN_KEY) {
        return key.contains_value(REG_APP_NAME);
    }
    false
}

// 设置开机自启
fn enable_autostart(exe_path: &str) -> std::io::Result<()> {
    let key = CURRENT_USER.open(REG_RUN_KEY)?;
    key.set_string(REG_APP_NAME, exe_path)?;
    Ok(())
}

// 取消开机自启
fn disable_autostart() -> std::io::Result<()> {
    let key = CURRENT_USER.open(REG_RUN_KEY)?;
    key.remove_value(REG_APP_NAME)?;
    Ok(())
}

fn main() {
    let exe_path: PathBuf = std::env::current_exe().unwrap();
    let exe_str = exe_path.to_string_lossy().to_string();

    let battery_state = Arc::new(Mutex::new(0u8));
    let last_alert = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(3600))); // 初始允许提醒

    // 右键菜单
    let menu = Menu::new();
    let refresh_item = MenuItem::new("🔄 刷新电量", true);
    let auto_start_item = if is_autostart_enabled() {
        MenuItem::new("⏹️ 取消开机自启", true)
    } else {
        MenuItem::new("🚀 设置开机自启", true)
    };
    let exit_item = MenuItem::new("❌ 退出", true);

    menu.append(&refresh_item).unwrap();
    menu.append(&auto_start_item).unwrap();
    menu.append(&exit_item).unwrap();

    let tray = TrayIconBuilder::new()
        .with_tooltip("Logi‑Tray(G502): 正在读取电量...")
        .with_menu(Box::new(menu.clone()))
        .build()
        .unwrap();

    // 后台刷新线程
    let tray_clone = tray.clone();
    let batt_state_clone = Arc::clone(&battery_state);
    let alert_clone = Arc::clone(&last_alert);
    std::thread::spawn(move || loop {
        if let Some(pct) = get_g502_battery() {
            *batt_state_clone.lock().unwrap() = pct;
            tray_clone.set_tooltip(Some(format!("G502 Lightspeed 电量: {}%", pct)));
            // 切换托盘图标
            let _ = tray_clone.set_icon(Some(get_icon_by_battery(pct)));

            // 低电量提醒：<=20%，间隔至少30分钟才弹窗一次
            if pct <= 20 {
                let mut last = alert_clone.lock().unwrap();
                if last.elapsed() > Duration::from_secs(1800) {
                    // 发送Windows通知
                    let _ = Notification::new()
                        .summary("⚠️ G502低电量警告")
                        .body(&format!("当前电量仅剩 {}%，请尽快充电！", pct))
                        .show();
                    *last = Instant::now();
                }
            }
        } else {
            tray_clone.set_tooltip(Some("Logi‑Tray: 未检测到G502无线"));
        }
        std::thread::sleep(Duration::from_secs(60));
    });

    // 菜单事件循环
    let (tx, rx) = std::sync::mpsc::channel();
    let tray_clone2 = tray.clone();
    let auto_start_item_clone = auto_start_item.clone();
    tray_icon::spawn_event_loop(move |event| {
        match event {
            tray_icon::Event::MenuEvent(id) => {
                if id == refresh_item.id() {
                    if let Some(pct) = get_g502_battery() {
                        *battery_state.lock().unwrap() = pct;
                        tray.set_tooltip(Some(format!("G502 Lightspeed 电量: {}%", pct)));
                        let _ = tray.set_icon(Some(get_icon_by_battery(pct)));
                        // 手动刷新后也触发低电量检查
                        if pct <=20 {
                            let mut last = alert_clone.lock().unwrap();
                            if last.elapsed() > Duration::from_secs(1800) {
                                let _ = Notification::new()
                                    .summary("⚠️ G502低电量警告")
                                    .body(&format!("当前电量仅剩 {}%，请尽快充电！", pct))
                                    .show();
                                *last = Instant::now();
                            }
                        }
                    }
                } else if id == auto_start_item_clone.id() {
                    if is_autostart_enabled() {
                        let _ = disable_autostart();
                        tray_clone2.set_tooltip(Some("已取消开机自启"));
                    } else {
                        let _ = enable_autostart(&exe_str);
                        tray_clone2.set_tooltip(Some("已开启开机自启"));
                    }
                } else if id == exit_item.id() {
                    tx.send(()).unwrap();
                    tray_icon::stop_event_loop();
                }
            }
            _ => {}
        }
    });
    rx.recv().ok();
}
