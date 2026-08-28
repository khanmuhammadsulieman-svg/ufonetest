use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use serde_json::{json, Value};
use rand::Rng;
use sqlx::{SqlitePool, Row};
use chrono::{Utc, Duration, Timelike};

use teloxide::prelude::*;
use teloxide::types::{Update, Message, CallbackQuery, ChatId, UserId, Recipient, InputFile};

use ufone_sdk::{UfoneClient, ClaimType};


const BOT_TOKEN: &str = "8392432345:AAG8CT_EOY7miKctL9anaRObcc04wHI-tXw";


const ADMIN_IDS: [i64; 1] = [6738222509];


const IMG_URL: &str = "https://humenglish.com/wp-content/uploads/2025/10/telecom-sector-1024x597.webp";

const E_TICK: &str = "✅";
const E_UFONE: &str = "🟠"; 
const E_USERS: &str = "👥";


#[derive(Clone, Debug, PartialEq)]
enum UserState {
    Home,
    GetPhone,
    VerifyOtp { phone: String, device_id: String },
    ReloginOtp { phone: String, device_id: String },
    BroadcastMsg,
    AddChLink,
    AddChNamePub { chid: String, link: String },
    AddChNamePriv { link: String },
    AddChIdPriv { name: String, link: String },
}

struct BotState {
    db: SqlitePool,
    sdk: UfoneClient,
    modes: HashMap<i64, UserState>,
    session_logs: HashMap<i64, String>,
}

#[tokio::main]
async fn main() {
    println!("🚀 SYSTEM INITIALIZED: Ufone Rust Bot Engine active...");

    let db_pool = SqlitePool::connect("sqlite:data.db?mode=rwc").await.unwrap();
    init_db(&db_pool).await;

    let state = Arc::new(Mutex::new(BotState {
        db: db_pool,
        sdk: UfoneClient::new(),
        modes: HashMap::new(),
        session_logs: HashMap::new(),
    }));

    let bot = Bot::new(BOT_TOKEN);

    let handler = dptree::entry()
        .branch(Update::filter_message().endpoint(handle_message))
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![state])
        .build()
        .dispatch()
        .await;
}


async fn init_db(db: &SqlitePool) {
    sqlx::query("CREATE TABLE IF NOT EXISTS users(id TEXT PRIMARY KEY)").execute(db).await.unwrap();
    sqlx::query("CREATE TABLE IF NOT EXISTS channels(ch_id TEXT PRIMARY KEY, btn_name TEXT, link TEXT)").execute(db).await.unwrap();
    sqlx::query("CREATE TABLE IF NOT EXISTS ufone_sessions(uid TEXT, phone TEXT, device_id TEXT, token TEXT, subtoken TEXT, status TEXT, UNIQUE(uid, phone))").execute(db).await.unwrap();
    sqlx::query("CREATE TABLE IF NOT EXISTS stats(key TEXT PRIMARY KEY, val INTEGER)").execute(db).await.unwrap();
    
    let stats_keys = ["today_spin", "today_daily", "total_spin", "total_daily"];
    for key in stats_keys {
        sqlx::query("INSERT OR IGNORE INTO stats VALUES (?, 0)").bind(key).execute(db).await.unwrap();
    }
}

fn safe_json_str(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => "".to_string(),
    }
}


fn append_to_session_log(logs: &mut HashMap<i64, String>, uid: i64, step_tag: &str, req: &Value, res: &Value) {
    let entry = logs.entry(uid).or_insert_with(String::new);
    let req_pretty = serde_json::to_string_pretty(req).unwrap_or_else(|_| "{}".to_string());
    let res_pretty = serde_json::to_string_pretty(res).unwrap_or_else(|_| "{}".to_string());
    
    entry.push_str(&format!(
        "==================================================\n\
         📌 STEP ACTION: {}\n\
         ==================================================\n\
         👉 OUTBOUND REQUEST PAYLOAD:\n{}\n\n\
         👈 INBOUND SERVER RESPONSE:\n{}\n\n\n",
        step_tag, req_pretty, res_pretty
    ));
}


fn get_pkt_reverse_timer() -> String {
    let pkt_time = Utc::now() + Duration::hours(5);
    let hours_left = 23 - pkt_time.hour();
    let mins_left = 59 - pkt_time.minute();
    format!("{:02}h {:02}m", hours_left, mins_left)
}


async fn send_vip_menu(chat_id: i64, text: &str, keyboard: Value, del_msg_id: Option<i32>) {
    let client = reqwest::Client::new();
    
    if let Some(msg_id) = del_msg_id {
        let del_url = format!("https://api.telegram.org/bot{}/deleteMessage", BOT_TOKEN);
        let _ = client.post(&del_url).json(&json!({"chat_id": chat_id, "message_id": msg_id})).send().await;
    }

    let url = format!("https://api.telegram.org/bot{}/sendPhoto", BOT_TOKEN);
    let payload = json!({
        "chat_id": chat_id,
        "photo": IMG_URL,
        "caption": text,
        "parse_mode": "HTML",
        "reply_markup": { "inline_keyboard": keyboard }
    });
    let _ = client.post(&url).json(&payload).send().await;
}


fn get_home_text() -> String {
    format!(
        "{} <b>UFONE REWARD CLAIM BOT</b> {} \n\
         ━━━━━━━━━━━━━━━ \n\n\
         {} <b>Daily Claim Rewards</b> {}\n\
         {} Add Multiple Numbers {} \n\
         {} <b>Every Friday 3GB Claim</b> {}\n\
         {} <b>Spin & Daily Rewards Both</b> {}\n\
         {} <b>Logout Your Number Easily</b> {}\n\
         ━━━━━━━━━━━━━━━",
        E_UFONE, E_UFONE, E_USERS, E_TICK, E_USERS, E_TICK, E_USERS, E_TICK, E_USERS, E_TICK, E_USERS, E_TICK
    )
}

fn get_start_text() -> String {
    format!(
        "{} <b>UFONE REWARD CLAIM BOT</b> {} \n\
         ━━━━━━━━━━━━━━━ \n\n\
         Welcome to Ufone Reward Claim Engine. Use the menu options below to link accounts and automate reward claiming.",
        E_UFONE, E_UFONE
    )
}

fn gen_device_id() -> String {
    let mut rng = rand::thread_rng();
    (0..16).map(|_| format!("{:x}", rng.gen::<u8>() % 16)).collect()
}

fn get_db_uid(uid: i64) -> String {
    if ADMIN_IDS.contains(&uid) { "ADMIN_SHARED_DB".to_string() } else { uid.to_string() }
}

async fn check_join(bot: &Bot, uid: i64, db: &SqlitePool) -> (bool, Value) {
    let rows = sqlx::query("SELECT ch_id, btn_name, link FROM channels").fetch_all(db).await.unwrap();
    let mut buttons = Vec::new();
    let mut tele_joined = true;

    if rows.is_empty() {
        return (true, json!([]));
    }

    for row in rows {
        let ch_id: String = row.get("ch_id");
        let btn_name: String = row.get("btn_name");
        let link: String = row.get("link");

        let mut row_style = "success";
        let mut icon = "✅";

        if let Ok(chat_id_parsed) = ch_id.parse::<i64>() {
            match bot.get_chat_member(ChatId(chat_id_parsed), UserId(uid as u64)).await {
                Ok(m) => {
                    let status_str = format!("{:?}", m.status()).to_lowercase();
                    if status_str.contains("left") || status_str.contains("kicked") || status_str.contains("banned") {
                        tele_joined = false; row_style = "danger"; icon = "❌";
                    }
                }
                _ => { tele_joined = false; row_style = "danger"; icon = "❌"; }
            }
        } else {
            
            match bot.get_chat_member(Recipient::ChannelUsername(ch_id.clone()), UserId(uid as u64)).await {
                Ok(m) => {
                    let status_str = format!("{:?}", m.status()).to_lowercase();
                    if status_str.contains("left") || status_str.contains("kicked") || status_str.contains("banned") {
                        tele_joined = false; row_style = "danger"; icon = "❌";
                    }
                }
                _ => { tele_joined = false; row_style = "danger"; icon = "❌"; }
            }
        }
        buttons.push(json!([{"text": format!("{} {}", icon, btn_name), "url": link, "style": row_style}]));
    }

    (tele_joined, Value::Array(buttons))
}

async fn show_home(chat_id: i64, uid: i64, db: &SqlitePool, del_id: Option<i32>) {
    let db_uid = get_db_uid(uid);
    let rows = sqlx::query("SELECT phone, status FROM ufone_sessions WHERE uid = ?").bind(db_uid).fetch_all(db).await.unwrap();
    let mut buttons = Vec::new();

    for row in rows {
        let phone: String = row.get("phone");
        let status: String = row.get("status");
        let style = if status == "active" { "success" } else { "danger" };
        buttons.push(json!([{"text": format!("📲 {}", phone), "callback_data": format!("num_{}", phone), "style": style}]));
    }

    buttons.push(json!([{"text": "➕ Add Number", "callback_data": "add_num", "style": "primary"}]));
    if ADMIN_IDS.contains(&uid) {
        buttons.push(json!([{"text": "👑 Admin Panel", "callback_data": "admin", "style": "primary"}]));
    }

    send_vip_menu(chat_id, &get_home_text(), Value::Array(buttons), del_id).await;
}


async fn handle_message(bot: Bot, msg: Message, state: Arc<Mutex<BotState>>) -> ResponseResult<()> {
    let uid = match &msg.from {
        Some(user) => user.id.0 as i64,
        None => return Ok(()),
    };
    let text = msg.text().unwrap_or("").trim().to_string();

    if text == "/log" {
        let s = state.lock().await;
        if let Some(log_data) = s.session_logs.get(&uid) {
            let document = InputFile::memory(log_data.clone().into_bytes()).file_name("ufone_lifecycle_dump.txt");
            let _ = bot.send_document(msg.chat.id, document).caption("📋 Here is your custom on-demand raw session dump.").await;
        } else {
            let _ = bot.send_message(msg.chat.id, "❌ No data logs captured yet in this current session.").await;
        }
        return Ok(());
    }

    if text == "/start" {
        let mut s = state.lock().await;
        s.modes.insert(uid, UserState::Home);
        sqlx::query("INSERT OR IGNORE INTO users VALUES (?)").bind(uid.to_string()).execute(&s.db).await.unwrap();

        let (joined, mut kb) = check_join(&bot, uid, &s.db).await;

        if !joined {
            if let Some(arr) = kb.as_array_mut() {
                arr.push(json!([{"text": "🟢 Check Join", "callback_data": "check_join", "style": "primary"}]));
                if ADMIN_IDS.contains(&uid) {
                    arr.push(json!([{"text": "👑 Admin Panel", "callback_data": "admin", "style": "primary"}]));
                }
            }
            send_vip_menu(uid, &get_start_text(), kb, None).await;
            return Ok(());
        }

        show_home(uid, uid, &s.db, None).await;
        return Ok(());
    }

    let current_mode = {
        let s = state.lock().await;
        s.modes.get(&uid).cloned().unwrap_or(UserState::Home)
    };

    match current_mode {
        UserState::GetPhone => {
            if text.len() != 11 || !text.starts_with("03") {
                bot.send_message(msg.chat.id, "❌ Invalid Format. Use 03XXXXXXXXX.").await?;
                return Ok(());
            }
            let device_id = gen_device_id();
            let mut s = state.lock().await;
            
            let req_dump = json!({"phone": text, "device_id": device_id});
            if let Ok(res) = s.sdk.send_otp(&text, &device_id).await {
                append_to_session_log(&mut s.session_logs, uid, "SEND_OTP_REQUEST", &req_dump, &res);

                if res["success"].as_bool().unwrap_or(false) {
                    s.modes.insert(uid, UserState::VerifyOtp { phone: text.clone(), device_id });
                    let kb = json!([ [{"text": "❌ Cancel", "callback_data": "home", "style": "danger"}] ]);
                    send_vip_menu(uid, &format!("🌟 <b>OTP Sent!</b>\nEnter 6-Digit OTP for:\n{}", text), kb, Some(msg.id.0)).await;
                } else {
                    let kb = json!([ [{"text": "❌ Cancel", "callback_data": "home", "style": "danger"}] ]);
                    send_vip_menu(uid, "❌ Failed to send OTP.", kb, Some(msg.id.0)).await;
                }
            }
        }
        UserState::VerifyOtp { phone, device_id } | UserState::ReloginOtp { phone, device_id } => {
            let mut s = state.lock().await;
            
            let req_dump = json!({"phone": phone, "otp": text, "device_id": device_id});
            if let Ok(res) = s.sdk.verify_otp(&phone, &text, &device_id).await {
                append_to_session_log(&mut s.session_logs, uid, "VERIFY_OTP_RESPONSE", &req_dump, &res);

                let server_res = &res["server_response"];
                let resp_code = server_res["responseCode"].as_str().unwrap_or("");
                if resp_code == "200" || resp_code == "0000" {
                    
                    let mut token = "".to_string();
                    let mut subtoken = "".to_string();
                    
                    if let Some(resp_str_raw) = server_res["responseString"].as_str() {
                        if let Ok(resp_json) = serde_json::from_str::<Value>(resp_str_raw) {
                            token = resp_json["customerDetails"]["token"].as_str().unwrap_or("").to_string();
                            subtoken = resp_json["customerDetails"]["subToken"].as_str().unwrap_or("").to_string();
                        }
                    }

                    if !token.is_empty() && !subtoken.is_empty() {
                        let db_uid = get_db_uid(uid);
                        sqlx::query("INSERT OR REPLACE INTO ufone_sessions VALUES (?, ?, ?, ?, ?, 'active')")
                            .bind(db_uid).bind(&phone).bind(&device_id).bind(&token).bind(&subtoken)
                            .execute(&s.db).await.unwrap();

                        s.modes.insert(uid, UserState::Home);
                        let kb = json!([ [{"text": "🔙 Go to Dashboard", "callback_data": format!("num_{}", phone), "style": "primary"}] ]);
                        send_vip_menu(uid, "✅ <b>Login Successful!</b>", kb, Some(msg.id.0)).await;
                    } else {
                        let kb = json!([ [{"text": "❌ Cancel", "callback_data": "home", "style": "danger"}] ]);
                        send_vip_menu(uid, "❌ <b>Token Parse Error:</b> Server response structure mismatch.", kb, Some(msg.id.0)).await;
                    }
                } else {
                    let kb = json!([ 
                        [{"text": "🔄 Resend OTP", "callback_data": format!("resend_otp_{}", phone), "style": "primary"}],
                        [{"text": "❌ Cancel", "callback_data": "home", "style": "danger"}]
                    ]);
                    send_vip_menu(uid, "❌ <b>Invalid OTP or Session Expired.</b>", kb, Some(msg.id.0)).await;
                }
            }
        }
        UserState::BroadcastMsg if ADMIN_IDS.contains(&uid) => {
            let s = state.lock().await;
            let users = sqlx::query("SELECT id FROM users").fetch_all(&s.db).await.unwrap();
            bot.send_message(msg.chat.id, format!("🌟 Broadcasting to {} users...", users.len())).await?;
            
            for u in users {
                let user_id: String = u.get("id");
                if let Ok(id_parsed) = user_id.parse::<i64>() {
                    let _ = bot.send_message(ChatId(id_parsed), format!("🌟 <b>Admin Broadcast</b>\n\n{}", text)).parse_mode(teloxide::types::ParseMode::Html).await;
                }
            }
            bot.send_message(msg.chat.id, "✅ Broadcast Complete!").await?;
        }
        UserState::AddChLink if ADMIN_IDS.contains(&uid) => {
            let mut s = state.lock().await;
            if text.contains("+") || text.contains("joinchat") {
                s.modes.insert(uid, UserState::AddChNamePriv { link: text });
                bot.send_message(msg.chat.id, "🌟 Private Link Detected.\nEnter Button Name:").await?;
            } else {
                let username = text.split('/').last().unwrap_or("").replace("@", "");
                s.modes.insert(uid, UserState::AddChNamePub { chid: format!("@{}", username), link: format!("https://t.me/{}", username) });
                bot.send_message(msg.chat.id, format!("🌟 Public Link Detected (@{}).\nEnter Button Name:", username)).await?;
            }
        }
        UserState::AddChNamePub { chid, link } if ADMIN_IDS.contains(&uid) => {
            let mut s = state.lock().await;
            sqlx::query("INSERT OR REPLACE INTO channels VALUES (?, ?, ?)")
                .bind(&chid).bind(&text).bind(&link).execute(&s.db).await.unwrap();
            s.modes.insert(uid, UserState::Home);
            bot.send_message(msg.chat.id, "✅ Public Channel Saved!").await?;
        }
        UserState::AddChNamePriv { link } if ADMIN_IDS.contains(&uid) => {
            let mut s = state.lock().await;
            s.modes.insert(uid, UserState::AddChIdPriv { name: text, link });
            bot.send_message(msg.chat.id, "Enter Channel ID (e.g. -10012345678):").await?;
        }
        UserState::AddChIdPriv { name, link } if ADMIN_IDS.contains(&uid) => {
            let mut s = state.lock().await;
            sqlx::query("INSERT OR REPLACE INTO channels VALUES (?, ?, ?)")
                .bind(&text).bind(&name).bind(&link).execute(&s.db).await.unwrap();
            s.modes.insert(uid, UserState::Home);
            bot.send_message(msg.chat.id, "✅ Private Channel Saved!").await?;
        }
        _ => {}
    }

    Ok(())
}


async fn handle_callback(bot: Bot, q: CallbackQuery, state: Arc<Mutex<BotState>>) -> ResponseResult<()> {
    let uid = q.from.id.0 as i64;
    let data = q.data.unwrap_or_default();
    let msg_id = q.message.map(|m| m.id().0);

    if data == "check_join" {
        let s = state.lock().await;
        let (joined, mut kb) = check_join(&bot, uid, &s.db).await;
        
        if joined {
            show_home(uid, uid, &s.db, msg_id).await;
        } else {
            if let Some(arr) = kb.as_array_mut() {
                arr.push(json!([{"text": "🟢 Check Join", "callback_data": "check_join", "style": "primary"}]));
            }
            send_vip_menu(uid, &get_home_text(), kb, msg_id).await;
        }
    }
    else if data == "home" {
        let s = state.lock().await;
        show_home(uid, uid, &s.db, msg_id).await;
    }
    else if data == "add_num" {
        let mut s = state.lock().await;
        s.modes.insert(uid, UserState::GetPhone);
        let kb = json!([ [{"text": "❌ Cancel", "callback_data": "home", "style": "danger"}] ]);
        send_vip_menu(uid, "🌟 <b>Enter Ufone Number (e.g. 03XXXXXXXX):</b>", kb, msg_id).await;
    }
    else if data.starts_with("resend_otp_") {
        let phone = data.split('_').nth(2).unwrap_or("");
        let device_id = gen_device_id();
        let mut s = state.lock().await;
        s.modes.insert(uid, UserState::VerifyOtp { phone: phone.to_string(), device_id: device_id.clone() });
        
        if let Ok(res) = s.sdk.send_otp(phone, &device_id).await {
            let kb = json!([ [{"text": "❌ Cancel", "callback_data": "home", "style": "danger"}] ]);
            if res["success"].as_bool().unwrap_or(false) {
                send_vip_menu(uid, &format!("🌟 <b>OTP Re-Sent!</b>\nEnter 6-Digit OTP for:\n{}", phone), kb, msg_id).await;
            } else {
                send_vip_menu(uid, "❌ Failed to resend OTP.", kb, msg_id).await;
            }
        }
    }
    else if data.starts_with("num_") {
        let phone = data.split('_').nth(1).unwrap_or("");
        let mut s = state.lock().await;
        let db_uid = get_db_uid(uid);

        let row = sqlx::query("SELECT device_id, token, subtoken, status FROM ufone_sessions WHERE uid = ? AND phone = ?")
            .bind(db_uid).bind(phone).fetch_optional(&s.db).await.unwrap();

        if let Some(r) = row {
            let device_id: String = r.get("device_id");
            let token: String = r.get("token");
            let subtoken: String = r.get("subtoken");
            let mut status: String = r.get("status");

            let (mut bal, mut net, mut mins, mut sms, mut adv_avl) = ("0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(), false);

            if status == "active" {
                let req_dump = json!({"phone": phone, "deviceid": device_id, "token": token, "subtoken": subtoken});
                if let Ok(usr_res) = s.sdk.get_user_details(phone, &device_id, &token, &subtoken).await {
                    
                    append_to_session_log(&mut s.session_logs, uid, "DASHBOARD_USER_DETAILS", &req_dump, &usr_res);

                    let resp_code = usr_res["server_response"]["responseCode"].as_str().unwrap_or("");
                    if resp_code == "0005" || resp_code == "401" {
                        sqlx::query("UPDATE ufone_sessions SET status='expired' WHERE phone=?").bind(phone).execute(&s.db).await.unwrap();
                        status = "expired".to_string();
                    } else if usr_res["success"].as_bool().unwrap_or(false) {
                        if let Some(resp_str_raw) = usr_res["server_response"]["responseString"].as_str() {
                            if let Ok(resp_str) = serde_json::from_str::<Value>(resp_str_raw) {
                                bal = safe_json_str(&resp_str["balanceDetails"]["balance"]);
                                if let Some(usage_arr) = resp_str["cumulativeUsage"].as_array() {
                                    for u in usage_arr {
                                        let u_type = u["type"].as_str().unwrap_or("");
                                        let rem = safe_json_str(&u["remaining"]);
                                        if u_type == "data" { net = rem; }
                                        else if u_type == "custom_mins" || u_type == "mins" { mins = rem; }
                                        else if u_type == "sms" { sms = rem; }
                                    }
                                }
                            }
                        }
                    }
                }
                
                if status == "active" {
                    if let Ok(adv_res) = s.sdk.check_advance(phone, &device_id, &token, &subtoken).await {
                        append_to_session_log(&mut s.session_logs, uid, "ADVANCE_CHECK", &json!({"phone": phone}), &adv_res);

                        if let Some(adv_raw) = adv_res["server_response"]["responseString"].as_str() {
                            if let Ok(adv_json) = serde_json::from_str::<Value>(adv_raw) {
                                if adv_json["status"].as_str().unwrap_or("") == "success" { adv_avl = true; }
                            }
                        }
                    }
                }
            }

            let status_txt = if status == "active" { "Active ✅" } else { "Session Expired ❌" };
            let adv_icon = if adv_avl { "✅" } else { "❌" };

            let txt = format!(
                "<blockquote>🌟 <b>VIP DASHBOARD</b> 🌟\n\n\
                 🔢 <b>Number:</b> {}\n\n\
                 🌟 <b>Status:</b> {}\n\n\
                 💰 <b>Advance Available:</b> {}\n\
                 🌟 <b>Balance:</b> Rs {}\n\n\
                 🌟 <b>Internet:</b> {} GB\n\
                 🌟 <b>Calls:</b> {} MINS\n\
                 🌟 <b>SMS:</b> {} SMS</blockquote>",
                phone, status_txt, adv_icon, bal, net, mins, sms
            );

            let mut kb = Vec::new();
            if status == "active" {
                kb.push(json!([{"text": "🎯 Spin The Wheel", "callback_data": format!("d_spin_{}", phone), "style": "primary"}]));
                kb.push(json!([{"text": "🎁 Daily Reward", "callback_data": format!("d_daily_{}", phone), "style": "success"}]));
            } else {
                kb.push(json!([{"text": "🟢 Re-Login Account", "callback_data": format!("relogin_{}", phone), "style": "success"}]));
            }
            kb.push(json!([{"text": "🗑️ Delete Account", "callback_data": format!("del_{}", phone), "style": "danger"}]));
            kb.push(json!([{"text": "🔙 Home", "callback_data": "home", "style": "primary"}]));

            send_vip_menu(uid, &txt, Value::Array(kb), msg_id).await;
        }
    }
    else if data.starts_with("d_spin_") {
        let phone = data.split('_').nth(2).unwrap_or("");
        let mut s = state.lock().await;
        let db_uid = get_db_uid(uid);

        let row = sqlx::query("SELECT device_id, token, subtoken FROM ufone_sessions WHERE uid=? AND phone=?")
            .bind(db_uid).bind(phone).fetch_one(&s.db).await.unwrap();
        let (device_id, token, subtoken): (String, String, String) = (row.get(0), row.get(1), row.get(2));
        
        let req_dump = json!({"phone": phone, "deviceid": device_id, "token": token, "subtoken": subtoken});

        match s.sdk.check_spin_info(phone, &device_id, &token, &subtoken).await {
            Ok(spin_res) => {
                append_to_session_log(&mut s.session_logs, uid, "SPIN_WHEEL_CHECK", &req_dump, &spin_res);

                let mut kb = Vec::new();
                let data_str = if spin_res["responseString"].is_string() {
                    let raw_str = spin_res["responseString"].as_str().unwrap_or("{}");
                    serde_json::from_str::<Value>(raw_str).unwrap_or(json!({}))
                } else {
                    spin_res["responseString"].clone()
                };

                let claim_status = safe_json_str(&data_str["todayClaimedStatus"]);
                
                if let Some(models) = data_str["model"].as_array() {
                    for item in models {
                        let val = item["value"].as_str().unwrap_or("");
                        let r_type = item["rewardType"].as_str().unwrap_or("");
                        if r_type == "tryagain" || val.to_lowercase().contains("try again") { continue; }
                        
                        if r_type == "reward" {
                            let apid = safe_json_str(&item["apId"]);
                            if claim_status == "1" {
                                kb.push(json!([{"text": format!("🎁 {}", val), "callback_data": format!("c_spin_{}_{}_{}", phone, apid, val), "style": "primary"}]));
                            } else {
                                kb.push(json!([{"text": format!("🔒 {}", val), "callback_data": "ignore", "style": "primary"}]));
                            }
                        }
                    }
                }

                if claim_status == "0" {
                    let timer_str = get_pkt_reverse_timer();
                    let mid_idx = kb.len() / 2;
                    kb.insert(mid_idx, json!([{"text": format!("⏳ Next Spin In: {}", timer_str), "callback_data": "ignore", "style": "danger"}]));
                }

                kb.push(json!([{"text": "🔙 Back", "callback_data": format!("num_{}", phone), "style": "danger"}]));
                send_vip_menu(uid, "<blockquote>🌟 <b>Spin The Wheel</b>\n\n🎯 <i>Select your reward below:</i></blockquote>", Value::Array(kb), msg_id).await;
            }
            Err(e) => {
                let kb = json!([ [{"text": "🔙 Back to Dashboard", "callback_data": format!("num_{}", phone), "style": "danger"}] ]);
                send_vip_menu(uid, &format!("❌ <b>SDK Request Error:</b> {}", e), kb, msg_id).await;
            }
        }
    }
    else if data.starts_with("d_daily_") {
        let phone = data.split('_').nth(2).unwrap_or("");
        let mut s = state.lock().await;
        let db_uid = get_db_uid(uid);

        let row = sqlx::query("SELECT device_id, token, subtoken FROM ufone_sessions WHERE uid=? AND phone=?")
            .bind(db_uid).bind(phone).fetch_one(&s.db).await.unwrap();
        let (device_id, token, subtoken): (String, String, String) = (row.get(0), row.get(1), row.get(2));
        
        let req_dump = json!({"phone": phone, "deviceid": device_id, "token": token, "subtoken": subtoken});

        match s.sdk.check_daily_info(phone, &device_id, &token, &subtoken).await {
            Ok(daily_res) => {
                append_to_session_log(&mut s.session_logs, uid, "DAILY_REWARD_CHECK", &req_dump, &daily_res);

                let mut kb = Vec::new();
                let data_str = if daily_res["responseString"].is_string() {
                    let raw_str = daily_res["responseString"].as_str().unwrap_or("{}");
                    serde_json::from_str::<Value>(raw_str).unwrap_or(json!({}))
                } else {
                    daily_res["responseString"].clone()
                };

                let claim_status = safe_json_str(&data_str["todayClaimedStatus"]);
                let day_count = safe_json_str(&data_str["dayCount"]);

                if let Some(day_list) = data_str["dayList"].as_array() {
                    for item in day_list {
                        let d_id = safe_json_str(&item["dayIdentifier"]);
                        let val = item["value"].as_str().unwrap_or("");
                        
                        if claim_status == "1" && d_id == day_count {
                            kb.push(json!([{"text": format!("🎁 Claim Day {} - {}", d_id, val), "callback_data": format!("c_daily_{}_{}_{}", phone, d_id, val), "style": "success"}]));
                        } else if claim_status == "0" && d_id == day_count {
                            let timer_str = get_pkt_reverse_timer();
                            kb.push(json!([{"text": format!("⏳ Day {} In: {}", d_id, timer_str), "callback_data": "ignore", "style": "danger"}]));
                        } else {
                            kb.push(json!([{"text": format!("🔒 Day {} - {}", d_id, val), "callback_data": "ignore", "style": "primary"}]));
                        }
                    }
                }
                kb.push(json!([{"text": "🔙 Back", "callback_data": format!("num_{}", phone), "style": "danger"}]));
                send_vip_menu(uid, "<blockquote>🌟 <b>Daily Reward</b>\n\n📅 <i>Your 7-Days Streak:</i></blockquote>", Value::Array(kb), msg_id).await;
            }
            Err(e) => {
                let kb = json!([ [{"text": "🔙 Back to Dashboard", "callback_data": format!("num_{}", phone), "style": "danger"}] ]);
                send_vip_menu(uid, &format!("❌ <b>SDK Request Error:</b> {}", e), kb, msg_id).await;
            }
        }
    }
    else if data.starts_with("c_spin_") || data.starts_with("c_daily_") {
        let parts: Vec<&str> = data.split('_').collect();
        let req_type = parts[1];
        let phone = parts[2];
        let param_id = parts[3];
        let val = parts[4];

        let mut s = state.lock().await;
        let db_uid = get_db_uid(uid);
        let row = sqlx::query("SELECT device_id, token, subtoken FROM ufone_sessions WHERE uid=? AND phone=?")
            .bind(db_uid).bind(phone).fetch_one(&s.db).await.unwrap();
        let (device_id, token, subtoken): (String, String, String) = (row.get(0), row.get(1), row.get(2));

        let claim_enum = if req_type == "spin" {
            ClaimType::SpinTheWheel { ap_id: param_id.to_string() }
        } else {
            ClaimType::DailyReward { day: param_id.to_string() }
        };

        let mut txt = format!("<blockquote>🌟 <b>Claim Result</b>\n\n🔢 <b>Number:</b>\n {}\n\n🎁 <b>Reward:</b> {}\n\n", phone, val);
        if let Ok(res) = s.sdk.claim_reward(phone, &device_id, &token, &subtoken, val, claim_enum).await {
            append_to_session_log(&mut s.session_logs, uid, &format!("CLAIM_REWARD_EXECUTE_{}", req_type), &json!({"phone": phone, "param_id": param_id, "val": val}), &res);

            if res["responseCode"] == "200" || res["status"] == "success" {
                txt += "✅ <b>Status:</b> Success!</blockquote>";
                let stat_key = if req_type == "spin" { "today_spin" } else { "today_daily" };
                let stat_total = if req_type == "spin" { "total_spin" } else { "total_daily" };
                sqlx::query("UPDATE stats SET val = val + 1 WHERE key=? OR key=?").bind(stat_key).bind(stat_total).execute(&s.db).await.unwrap();
            } else {
                let desc = res["responseDesc"].as_str().unwrap_or("Failed!");
                txt += &format!("❌ <b>Status:</b> {}</blockquote>", desc);
            }
        }
        let kb = json!([ [{"text": "🔙 Back to Dashboard", "callback_data": format!("num_{}", phone), "style": "primary"}] ]);
        send_vip_menu(uid, &txt, kb, msg_id).await;
    }
    else if data.starts_with("del_") {
        let phone = data.split('_').nth(1).unwrap_or("");
        let s = state.lock().await;
        let db_uid = get_db_uid(uid);
        sqlx::query("DELETE FROM ufone_sessions WHERE uid=? AND phone=?").bind(db_uid).bind(phone).execute(&s.db).await.unwrap();
        show_home(uid, uid, &s.db, msg_id).await;
    }
    else if data.starts_with("relogin_") {
        let phone = data.split('_').nth(1).unwrap_or("");
        let device_id = gen_device_id();
        let mut s = state.lock().await;
        s.modes.insert(uid, UserState::ReloginOtp { phone: phone.to_string(), device_id: device_id.clone() });
        
        if let Ok(res) = s.sdk.send_otp(phone, &device_id).await {
            let kb = json!([ [{"text": "❌ Cancel", "callback_data": "home", "style": "danger"}] ]);
            if res["success"].as_bool().unwrap_or(false) {
                send_vip_menu(uid, &format!("🌟 <b>OTP Sent!</b>\nEnter 6-Digit OTP for:\n{}", phone), kb, msg_id).await;
            } else {
                send_vip_menu(uid, "❌ Failed to send OTP.", kb, msg_id).await;
            }
        }
    }
    
    else if data == "admin" && ADMIN_IDS.contains(&uid) {
        let kb = json!([
            [{"text": "📲 Connected Numbers", "callback_data": "admin_nums", "style": "success"}],
            [{"text": "➕ Add Channel", "callback_data": "admin_add_ch", "style": "primary"}, {"text": "🗑️ Manage Channel", "callback_data": "admin_man_ch", "style": "primary"}],
            [{"text": "📢 Broadcast", "callback_data": "admin_broadcast", "style": "primary"}, {"text": "🌟 System Stats", "callback_data": "admin_stats", "style": "success"}],
            [{"text": "❌ Close", "callback_data": "home", "style": "danger"}]
        ]);
        send_vip_menu(uid, "<blockquote>🌟 <b>Admin Dashboard</b></blockquote>", kb, msg_id).await;
    }
    else if data == "admin_nums" && ADMIN_IDS.contains(&uid) {
        let s = state.lock().await;
        let rows = sqlx::query("SELECT phone FROM ufone_sessions").fetch_all(&s.db).await.unwrap();
        let mut kb = Vec::new();
        let mut chunk = Vec::new();

        for row in rows {
            let phone: String = row.get("phone");
            chunk.push(json!({"text": format!("📲 {}", phone), "callback_data": format!("num_{}", phone), "style": "primary"}));
            if chunk.len() == 2 {
                kb.push(Value::Array(chunk.clone()));
                chunk.clear();
            }
        }
        if !chunk.is_empty() { kb.push(Value::Array(chunk)); }
        kb.push(json!([{"text": "🔙 Back", "callback_data": "admin", "style": "danger"}]));
        send_vip_menu(uid, "<blockquote>🌟 <b>All Connected Numbers:</b></blockquote>", Value::Array(kb), msg_id).await;
    }
    else if data == "admin_add_ch" && ADMIN_IDS.contains(&uid) {
        let mut s = state.lock().await;
        s.modes.insert(uid, UserState::AddChLink);
        let kb = json!([ [{"text": "❌ Cancel", "callback_data": "admin", "style": "danger"}] ]);
        send_vip_menu(uid, "🌟 <b>Step 1:</b> Send Channel Link or Username (e.g. <code>@MyChannel</code>):", kb, msg_id).await;
    }
    else if data == "admin_man_ch" && ADMIN_IDS.contains(&uid) {
        let s = state.lock().await;
        let rows = sqlx::query("SELECT ch_id, btn_name FROM channels").fetch_all(&s.db).await.unwrap();
        let mut kb = Vec::new();
        for r in rows {
            let cid: String = r.get("ch_id");
            let name: String = r.get("btn_name");
            kb.push(json!([{"text": format!("🗑️ {}", name), "callback_data": format!("chdel_{}", cid), "style": "danger"}]));
        }
        kb.push(json!([{"text": "🔙 Back", "callback_data": "admin", "style": "primary"}]));
        send_vip_menu(uid, "<blockquote>🌟 <b>Click a channel to Delete:</b></blockquote>", Value::Array(kb), msg_id).await;
    }
    else if data.starts_with("chdel_") && ADMIN_IDS.contains(&uid) {
        let cid = data.split('_').nth(1).unwrap_or("");
        let s = state.lock().await;
        sqlx::query("DELETE FROM channels WHERE ch_id=?").bind(cid).execute(&s.db).await.unwrap();
        let kb = json!([ [{"text": "🔙 Back", "callback_data": "admin", "style": "primary"}] ]);
        send_vip_menu(uid, "✅ Channel Deleted Successfully!", kb, msg_id).await;
    }
    else if data == "admin_broadcast" && ADMIN_IDS.contains(&uid) {
        let mut s = state.lock().await;
        s.modes.insert(uid, UserState::BroadcastMsg);
        let kb = json!([ [{"text": "❌ Cancel", "callback_data": "admin", "style": "danger"}] ]);
        send_vip_menu(uid, "🌟 <b>Send the message you want to broadcast to all users:</b>", kb, msg_id).await;
    }
    else if data == "admin_stats" && ADMIN_IDS.contains(&uid) {
        let s = state.lock().await;
        
        let u_cnt: i32 = sqlx::query("SELECT COUNT(*) FROM users").fetch_one(&s.db).await.unwrap().get(0);
        let n_cnt: i32 = sqlx::query("SELECT COUNT(*) FROM ufone_sessions").fetch_one(&s.db).await.unwrap().get(0);
        
        let mut stats_map = HashMap::new();
        let rows = sqlx::query("SELECT key, val FROM stats").fetch_all(&s.db).await.unwrap();
        for r in rows {
            let k: String = r.get("key");
            let v: i32 = r.get("val");
            stats_map.insert(k, v);
        }

        let txt = format!(
            "<blockquote>🌟 <b>System Stats</b>\n\n\
             👥 Total Users: {}\n\
             📲 Connected Numbers: {}\n\n\
             🟢 Today Spin Claims: {}\n\n\
             🟢 Today Daily Claims: {}\n\n\
             🎁 Total Spin Claims: {}\n\n\
             🎁 Total Daily Claims: {}</blockquote>",
            u_cnt, n_cnt, 
            stats_map.get("today_spin").unwrap_or(&0), stats_map.get("today_daily").unwrap_or(&0),
            stats_map.get("total_spin").unwrap_or(&0), stats_map.get("total_daily").unwrap_or(&0)
        );
        let kb = json!([ [{"text": "🔙 Back", "callback_data": "admin", "style": "primary"}] ]);
        send_vip_menu(uid, &txt, kb, msg_id).await;
    }

    let _ = bot.answer_callback_query(q.id.clone()).await;
    Ok(())
}
