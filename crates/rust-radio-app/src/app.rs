//! 主界面：侧栏 / 工具条 / 内容区 / 节目单抽屉 / 播放条 / 搜索浮层。

use crate::audit;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Stroke,
    StrokeKind, Vec2,
};
use rust_radio_audio::{Engine, PlayState, Source};
use rust_radio_core::{
    Client, Freshness, Line, Program, Province, Quality, Station, StationDetail, StationIndex,
    StreamKind, StreamTarget,
};

use crate::icons::{self, Icon};
use crate::images::Images;
use crate::store::Store;
use crate::{theme, widgets};

/// 每页 40 条：接口 pageSize 最小 21，直接按 40 取一页展示
const SEARCH_PAGE_SIZE: u32 = 40;

/// 自绘标题栏高度（所有平台统一）
const TITLEBAR_H: f32 = 36.0;
/// macOS：原生红绿灯占用的左侧宽度
const MAC_LIGHTS_W: f32 = 78.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Discover,
    National,
    All,
    Region,
    Favorites,
    Recent,
    Search,
    Settings,
}

impl View {
    fn title(&self) -> &'static str {
        match self {
            View::Discover => "发现",
            View::National => "央广频率",
            View::All => "全部电台",
            View::Region => "按地区",
            View::Favorites => "我的收藏",
            View::Recent => "最近播放",
            View::Search => "搜索",
            View::Settings => "设置",
        }
    }
}

#[derive(Clone)]
struct Now {
    station: Station,
    program_id: Option<String>,
    program_name: String,
    line_label: String,
    is_replay: bool,
    candidates: Vec<StreamTarget>,
    next_candidate: usize,
    retries: u32,
    error_handled: bool,
    started: Instant,
    duration: Option<f64>,
    position_hint: f64,
}

enum Msg {
    Provinces(Vec<Province>),
    National(Vec<Station>, Freshness),
    Index(Vec<Station>, StationIndex, Freshness),
    Province(u32, Vec<Station>),
    SearchDone(u64, String, u32, Vec<Station>, u64),
    SearchFail(u64, String),
    Resolved(String, Box<StationDetail>, Vec<StreamTarget>),
    Programs(Vec<Program>),
    Fail(String),
}

pub struct App {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    client: Arc<Client>,
    engine: Engine,
    images: Images,
    data_dir: PathBuf,
    store: Store,

    provinces: Vec<Province>,
    national: Vec<Station>,
    all: Vec<Station>,
    index: StationIndex,
    freshness: Freshness,
    by_province: HashMap<u32, Vec<Station>>,
    province_loading: HashMap<u32, Instant>,
    index_loading: bool,

    view: View,
    prov: u32,
    /// 已提交的搜索关键词
    query: String,
    /// 侧栏搜索框内容
    nav_query: String,
    /// 搜索结果页搜索框内容
    search_input: String,
    /// 进入搜索页时把焦点放到结果页输入框
    search_focus: bool,
    /// ⌘K：把焦点放到侧栏搜索框
    nav_search_focus: bool,
    results: Vec<Station>,
    /// 搜索请求代次，晚到的旧响应直接丢弃
    search_seq: u64,
    search_loading: bool,
    search_total: u64,
    /// 已加载到第几页（0 基，每页 40 条）
    search_page: u32,
    /// 总页数（按每页 40 条算）
    search_pages: u32,
    search_error: Option<String>,
    selected: Option<String>,
    detail: Option<StationDetail>,
    programs: Vec<Program>,
    program_date: chrono::NaiveDate,
    program_day: usize,
    program_loading: bool,
    program_scroll: Option<f32>,
    audit_t0: f64,
    audit_step: usize,
    audit_probed: bool,
    pending_replay: bool,
    live_check: u8,
    live_check_t: Instant,
    panel_open: bool,
    /// 进入迷你窗口前的右侧面板状态，还原时恢复
    panel_before_mini: Option<bool>,
    /// 缩小窗口前先画一帧迷你底色，避免旧界面被裁成“巨大图标”
    mini_pending: bool,
    /// 放大窗口前先清一帧，避免迷你界面被铺到大窗口里闪一下
    full_pending: bool,

    now: Option<Now>,
    volume: f32,
    muted: bool,
    sleep_until: Option<Instant>,
    sleep_minutes: u32,
    toast: Option<(String, Instant)>,
    start: Instant,
    progress_saved: Instant,
    resolve_pending: bool,
    pending_autoplay: bool,
    grid_mode: bool,
    prov_open: bool,
    seek_preview: Option<f64>,

    prev_state: PlayState,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        data_dir: PathBuf,
        autoplay: Option<String>,
        start_view: Option<String>,
    ) -> Self {
        let cache_dir = data_dir.join("cache");
        let client = Arc::new(Client::new(cache_dir.clone()).expect("client"));
        let engine = Engine::new().expect("audio engine");
        let store = Store::load(&data_dir);
        let panel_open = store.panel_open;
        let prov_open = store.prov_open;
        let store_volume = store.volume;
        let sleep_last = store.sleep_last_minutes;
        engine.set_volume(store.volume);

        let (tx, rx) = mpsc::channel();
        #[cfg(target_os = "macos")]
        crate::localize_macos_menu();

        let mut app = Self {
            tx,
            rx,
            client,
            engine,
            images: Images::new(cache_dir.join("images")),
            data_dir,
            store,
            detail: None,
            provinces: Vec::new(),
            national: Vec::new(),
            all: Vec::new(),
            index: StationIndex::default(),
            freshness: Freshness::Live,
            by_province: HashMap::new(),
            province_loading: HashMap::new(),
            index_loading: true,
            view: View::Discover,
            prov: 110000,
            query: String::new(),
            nav_query: String::new(),
            search_input: String::new(),
            search_focus: false,
            nav_search_focus: false,
            results: Vec::new(),
            search_seq: 0,
            search_loading: false,
            search_total: 0,
            search_page: 0,
            search_pages: 0,
            search_error: None,
            selected: None,
            programs: Vec::new(),
            program_date: chrono::Local::now().date_naive(),
            program_day: 0,
            program_loading: false,
            program_scroll: None,
            audit_t0: 0.0,
            audit_step: 0,
            audit_probed: false,
            pending_replay: false,
            live_check: 0,
            live_check_t: Instant::now(),
            panel_open,
            panel_before_mini: None,
            mini_pending: false,
            full_pending: false,
            now: None,
            volume: store_volume,
            muted: false,
            sleep_until: None,
            sleep_minutes: sleep_last,
            toast: None,
            start: Instant::now(),
            progress_saved: Instant::now(),
            resolve_pending: false,
            pending_autoplay: false,
            grid_mode: true,
            prov_open,
            seek_preview: None,
            prev_state: PlayState::Idle,
        };
        app.bootstrap(&cc.egui_ctx);
        if let Some(id) = autoplay {
            app.select_station(&id, true);
        }
        // `--view all` 之类：直接落到指定页面（也方便自动化截图）
        if std::env::args().any(|a| a == "--replay") {
            app.pending_replay = true;
        }
        if std::env::args().any(|a| a == "--list") {
            app.grid_mode = false;
        }
        if std::env::args().any(|a| a == "--check-live") {
            app.live_check = 1;
        }
        // `--search=关键词`：预填搜索框（自动化截图 / 手动核对）
        if let Some(kw) =
            std::env::args().find_map(|a| a.strip_prefix("--search=").map(str::to_string))
        {
            app.search_input = kw;
            app.view = View::Search;
            app.run_search();
        }
        if let Some(v) = start_view {
            app.view = match v.to_ascii_lowercase().as_str() {
                "discover" => View::Discover,
                "national" => View::National,
                "all" => View::All,
                "region" => View::Region,
                "favorites" => View::Favorites,
                "recent" => View::Recent,
                "search" => View::Search,
                "settings" => View::Settings,
                _ => app.view,
            };
            if app.view == View::All {
                app.panel_open = false;
            }
        }
        app
    }

    fn bootstrap(&mut self, ctx: &egui::Context) {
        let ctx2 = ctx.clone();
        self.spawn(move |c| match c.provinces() {
            Ok(p) => Msg::Provinces(p),
            Err(e) => Msg::Fail(e.ui_hint().into()),
        });
        let ctx3 = ctx.clone();
        self.spawn(move |c| match c.national() {
            Ok(list) => Msg::National(list, Freshness::Live),
            Err(e) => Msg::Fail(e.ui_hint().into()),
        });
        let ctx4 = ctx.clone();
        self.spawn(move |c| match c.all_stations(false) {
            Ok((list, fresh)) => {
                let index = StationIndex::build(&list);
                Msg::Index(list, index, fresh)
            }
            Err(e) => Msg::Fail(e.ui_hint().into()),
        });
        let _ = (ctx2, ctx3, ctx4);
        if let Some(last) = self.store.last_station.clone() {
            self.select_station(&last, false);
        }
    }

    fn spawn<F>(&self, f: F)
    where
        F: FnOnce(&Client) -> Msg + Send + 'static,
    {
        let client = self.client.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let msg = f(&client);
            let _ = tx.send(msg);
        });
    }

    // ------------------------------------------------------------ 消息

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Provinces(p) => self.provinces = p,
                Msg::National(list, f) => {
                    let first = list.first().map(|s| s.content_id.clone());
                    self.national = list;
                    self.freshness = f;
                    if self.selected.is_none() {
                        if let Some(id) = first {
                            self.select_station(&id, false);
                        }
                    }
                }
                Msg::Index(list, index, f) => {
                    self.all = list;
                    self.index = index;
                    self.freshness = f;
                    self.index_loading = false;
                    // 搜索先于索引返回时，补一次元数据
                    let ids: Vec<String> =
                        self.results.iter().map(|s| s.content_id.clone()).collect();
                    for (i, id) in ids.iter().enumerate() {
                        if let Some(local) = self.index.get(id) {
                            self.results[i] = local.clone();
                        }
                    }
                }
                Msg::Province(code, list) => {
                    self.province_loading.remove(&code);
                    self.by_province.insert(code, list);
                }
                Msg::SearchDone(seq, kw, page, items, total) => {
                    // 晚到的旧响应（关键词已改）直接丢弃
                    if seq == self.search_seq && kw == self.query {
                        self.search_loading = false;
                        self.search_total = total;
                        self.search_page = page;
                        self.search_pages = (total.div_ceil(SEARCH_PAGE_SIZE as u64) as u32).max(1);
                        self.results = items
                            .into_iter()
                            .map(|s| enrich_station(s, &self.index))
                            .collect();
                    }
                }
                Msg::SearchFail(seq, msg) => {
                    if seq == self.search_seq {
                        self.search_loading = false;
                        self.search_error = Some(msg);
                    }
                }
                Msg::Resolved(id, detail, cands) => {
                    self.resolve_pending = false;
                    if self.selected.as_deref() == Some(id.as_str()) {
                        if self.pending_autoplay {
                            self.begin_playback(id, *detail, cands);
                        } else {
                            let d = *detail;
                            if let Some(name) = d.living_program_name.clone() {
                                self.apply_live_program(&id, &name);
                            }
                            self.detail = Some(d);
                            self.resolve_pending = false;
                        }
                    }
                }
                Msg::Programs(list) => {
                    self.program_loading = false;
                    self.programs = list;
                    self.program_scroll = self.scroll_to_now();
                }
                Msg::Fail(msg) => {
                    self.resolve_pending = false;
                    self.index_loading = false;
                    self.toast(msg);
                }
            }
            ctx.request_repaint();
        }
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    // ------------------------------------------------------------ 选择与播放

    fn select_station(&mut self, id: &str, autoplay: bool) {
        self.selected = Some(id.to_string());
        self.pending_autoplay = autoplay;
        self.resolve_pending = true;
        self.program_loading = true;
        let id_owned = id.to_string();
        let id2 = id_owned.clone();
        let day = self.program_date.format("%Y-%m-%d").to_string();
        self.spawn(move |c| {
            let detail = c.detail(&id_owned);
            match detail {
                Ok(d) => {
                    let cands = c.stream_candidates(&d, Quality::High, Line::Auto);
                    Msg::Resolved(id_owned, Box::new(d), cands)
                }
                Err(e) => Msg::Fail(e.ui_hint().into()),
            }
        });
        self.spawn(move |c| match c.programs(&id2, &day) {
            Ok((list, _)) => Msg::Programs(list),
            Err(_) => Msg::Programs(Vec::new()),
        });
        if autoplay {
            self.store.last_station = Some(id.to_string());
        }
    }

    fn begin_playback(&mut self, id: String, detail: StationDetail, cands: Vec<StreamTarget>) {
        let Some(first) = cands.first().cloned() else {
            self.toast("该电台暂无可用播放地址");
            return;
        };
        let station = self.station_by_id(&id).unwrap_or_else(|| Station {
            content_id: id.clone(),
            title: detail.name.clone(),
            ..Default::default()
        });
        let program_name = detail
            .living_program_name
            .clone()
            .unwrap_or_else(|| station.now_playing().to_string());
        if let Some(name) = detail.living_program_name.clone() {
            self.apply_live_program(&id, &name);
        }
        self.detail = Some(detail.clone());
        self.engine.play(Source {
            url: first.url.clone(),
            kind: first.kind,
            title: station.title.clone(),
            subtitle: program_name.clone(),
            duration: None,
            start_at: 0.0,
        });
        self.store.push_recent(&station, "live", &program_name);
        self.store.last_station = Some(id.clone());
        self.save_store();
        self.now = Some(Now {
            station,
            program_id: None,
            program_name,
            line_label: first.label.clone(),
            is_replay: false,
            candidates: cands,
            next_candidate: 1,
            retries: 0,
            error_handled: false,
            started: Instant::now(),
            duration: None,
            position_hint: 0.0,
        });
    }

    fn play_program(&mut self, program: Program) {
        let Some(url) = program.replay_url().map(|s| s.to_string()) else {
            self.toast("这档节目没有回听地址");
            return;
        };
        let station = self
            .selected
            .as_ref()
            .and_then(|id| self.station_by_id(id))
            .or_else(|| self.now.as_ref().map(|n| n.station.clone()));
        let Some(station) = station else { return };
        let start_at = self
            .store
            .progress
            .get(&program.id)
            .copied()
            .filter(|p| *p > 5.0)
            .unwrap_or(0.0);
        let duration = program.duration_secs() as f64;
        self.engine.set_paused(false);
        self.engine.play(Source {
            url,
            kind: StreamKind::File,
            title: station.title.clone(),
            subtitle: program.program_name.clone(),
            duration: Some(duration),
            start_at,
        });
        self.store
            .push_recent(&station, "replay", &program.program_name);
        self.save_store();
        self.now = Some(Now {
            station,
            program_id: Some(program.id.clone()),
            program_name: program.program_name.clone(),
            line_label: "回听".into(),
            is_replay: true,
            candidates: Vec::new(),
            next_candidate: 0,
            retries: 0,
            error_handled: false,
            started: Instant::now(),
            duration: Some(duration),
            position_hint: start_at,
        });
        if start_at > 5.0 {
            self.toast(format!(
                "从上次听到的位置继续（{}）",
                widgets::fmt_time(start_at)
            ));
        }
    }

    /// 详情里的「正在播」比列表缓存新，同步回内存里的列表项
    fn apply_live_program(&mut self, id: &str, name: &str) {
        let subtitle = format!("正在直播：{name}");
        let update = |s: &mut Station| {
            s.subtitle = subtitle.clone();
        };
        if let Some(s) = self.national.iter_mut().find(|s| s.content_id == id) {
            update(s);
        }
        if let Some(s) = self.all.iter_mut().find(|s| s.content_id == id) {
            update(s);
        }
        for list in self.by_province.values_mut() {
            if let Some(s) = list.iter_mut().find(|s| s.content_id == id) {
                update(s);
            }
        }
    }

    fn station_by_id(&self, id: &str) -> Option<Station> {
        self.all
            .iter()
            .chain(self.national.iter())
            .chain(self.by_province.values().flatten())
            .find(|s| s.content_id == id)
            .cloned()
            .or_else(|| {
                self.store
                    .recents
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| Station {
                        content_id: r.id.clone(),
                        title: r.title.clone(),
                        tag_name: r.tag.clone(),
                        subtitle: r.program_name.clone(),
                        image: r.image.clone(),
                        province_code: 0,
                    })
            })
            .or_else(|| {
                self.store
                    .favorites
                    .iter()
                    .find(|f| f.id == id)
                    .map(|f| Station {
                        content_id: f.id.clone(),
                        title: f.title.clone(),
                        tag_name: f.tag.clone(),
                        subtitle: f.subtitle.clone(),
                        image: f.image.clone(),
                        province_code: 0,
                    })
            })
    }

    fn save_store(&mut self) {
        self.store.volume = self.volume;
        self.store.panel_open = self.panel_open;
        self.store.sleep_last_minutes = self.sleep_minutes;
        self.store.save(&self.data_dir);
    }

    // ------------------------------------------------------------ 播放状态机

    fn tick(&mut self, ctx: &egui::Context) {
        let snap = self.engine.snapshot();

        // 错误恢复：换线路 → 重新解析
        if snap.state == PlayState::Error {
            if let Some(now) = &mut self.now {
                if !now.error_handled && now.started.elapsed() > Duration::from_millis(600) {
                    now.error_handled = true;
                    if now.next_candidate < now.candidates.len() {
                        let next = now.candidates[now.next_candidate].clone();
                        now.next_candidate += 1;
                        now.line_label = next.label.clone();
                        let src = Source {
                            url: next.url,
                            kind: next.kind,
                            title: now.station.title.clone(),
                            subtitle: now.program_name.clone(),
                            duration: now.duration,
                            start_at: now.position_hint,
                        };
                        self.engine.play(src);
                        let label = next.label.clone();
                        self.toast(format!("线路异常，已切换到{label}"));
                    } else if now.retries < 2 && !now.is_replay {
                        now.retries += 1;
                        now.next_candidate = 1;
                        let id = now.station.content_id.clone();
                        self.toast("重新获取播放地址…");
                        self.spawn(move |c| match c.detail(&id) {
                            Ok(d) => {
                                let cands = c.stream_candidates(&d, Quality::High, Line::Auto);
                                Msg::Resolved(id, Box::new(d), cands)
                            }
                            Err(e) => Msg::Fail(e.ui_hint().into()),
                        });
                    } else {
                        let msg = snap.error.clone().unwrap_or_else(|| "播放失败".into());
                        self.toast(format!("播放中断：{msg}"));
                    }
                }
            }
        }
        if snap.state == PlayState::Playing && self.prev_state != PlayState::Playing {
            if let Some(now) = &mut self.now {
                now.error_handled = false;
                now.retries = 0;
            }
        }
        self.prev_state = snap.state;

        // 回听进度落盘
        if self.progress_saved.elapsed() > Duration::from_secs(5) {
            self.progress_saved = Instant::now();
            if let Some(now) = &self.now {
                if let Some(pid) = &now.program_id {
                    let pos = snap.position;
                    if pos > 3.0 {
                        self.store.progress.insert(pid.clone(), pos);
                        self.store.save(&self.data_dir);
                    }
                }
            }
        }

        // 睡眠定时
        if let Some(deadline) = self.sleep_until {
            if Instant::now() >= deadline {
                self.engine.stop();
                self.now = None;
                self.sleep_until = None;
                self.toast("睡眠定时结束，已停止播放");
            } else {
                ctx.request_repaint_after(Duration::from_millis(500));
            }
        }

        if self.engine.is_active() || self.resolve_pending {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    // ------------------------------------------------------------ 布局

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let w = theme::SIDEBAR_W;
        ui.set_width(w);
        let full = ui.max_rect();
        ui.painter()
            .rect_filled(full, CornerRadius::ZERO, theme::PANEL);
        ui.painter().line_segment(
            [
                Pos2::new(full.right() - 0.5, full.top()),
                Pos2::new(full.right() - 0.5, full.bottom()),
            ],
            Stroke::new(1.0, theme::white_alpha(16)),
        );

        ui.add_space(14.0);
        // 品牌放在导航栏顶部（标题栏只留窗口控件与拖拽区）
        let (brand_rect, _) = ui.allocate_exact_size(Vec2::new(w, 22.0), Sense::hover());
        {
            let p = ui.painter().clone();
            let mark = Rect::from_min_size(
                Pos2::new(brand_rect.left() + 18.0, brand_rect.top() + 2.0),
                Vec2::splat(18.0),
            );
            p.rect_filled(mark, CornerRadius::same(5), theme::ACCENT);
            icons::draw(&p, mark.shrink(2.0), Icon::Radio, theme::ON_ACCENT, 13.0);
            audit::text(
                &p,
                Pos2::new(mark.right() + 9.0, mark.center().y),
                Align2::LEFT_CENTER,
                "云听电台",
                theme::bold(14.0),
                theme::TEXT,
            );
        }
        ui.add_space(12.0);

        // 搜索入口：点按钮跳到搜索结果页，不再就地弹层
        let search_top = ui.cursor().top();
        let search_rect = Rect::from_min_size(
            Pos2::new(full.left() + 14.0, search_top),
            Vec2::new(w - 28.0, 34.0),
        );
        let active = self.view == View::Search;
        let btn_w = 30.0;
        let btn_rect = Rect::from_center_size(
            Pos2::new(
                search_rect.right() - 6.0 - btn_w * 0.5,
                search_rect.center().y,
            ),
            Vec2::new(btn_w, 26.0),
        );
        let field_rect = Rect::from_min_max(
            Pos2::new(search_rect.left() + 30.0, search_rect.top() + 4.0),
            Pos2::new(btn_rect.left() - 8.0, search_rect.bottom() - 4.0),
        );
        let hovered = ui.rect_contains_pointer(search_rect) || active;
        ui.painter().rect(
            search_rect,
            CornerRadius::same(theme::R_BTN),
            if hovered {
                theme::ELEV
            } else {
                Color32::from_rgb(0x07, 0x08, 0x0a)
            },
            Stroke::new(1.0, theme::white_alpha(if hovered { 40 } else { 26 })),
            StrokeKind::Inside,
        );
        icons::draw(
            ui.painter(),
            Rect::from_center_size(
                Pos2::new(search_rect.left() + 16.0, search_rect.center().y),
                Vec2::splat(14.0),
            ),
            Icon::Search,
            if active { theme::TEXT } else { theme::WEAK },
            15.0,
        );
        let text_rect = field_rect.translate(Vec2::new(0.0, -1.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let te = egui::TextEdit::singleline(&mut self.nav_query)
            .frame(egui::Frame::NONE)
            .hint_text(RichText::new("搜索电台").size(12.0).color(theme::WEAK))
            .text_color(theme::TEXT)
            .desired_width(text_rect.width())
            .margin(egui::Margin::ZERO)
            .align(Align2::LEFT_CENTER)
            .id(egui::Id::unique("nav-search-field"));
        let out = child.add_sized([text_rect.width(), text_rect.height()], te);
        if self.nav_search_focus {
            out.request_focus();
            self.nav_search_focus = false;
        }
        let submitted = out.lost_focus() && child.input(|i| i.key_pressed(egui::Key::Enter));

        // 搜索按钮：提交后跳到结果页
        let go = {
            let _r = btn_rect;
            audit::ctl("nav-search-go", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("nav-search-go"), Sense::click())
        };
        let fill = if go.hovered() {
            theme::ACCENT_HOVER
        } else {
            theme::ACCENT
        };
        ui.painter()
            .rect_filled(btn_rect, CornerRadius::same(7), fill);
        icons::draw(ui.painter(), btn_rect, Icon::Search, theme::ON_ACCENT, 14.0);
        if submitted || go.clicked() {
            self.submit_nav_search();
        }

        // 真正占用搜索框的高度（之前只画不占位，导致「发现」压在搜索框上）
        ui.allocate_space(Vec2::new(w, 34.0));
        ui.add_space(10.0);

        // 导航区：窗口变矮时只滚这一块，上面的搜索框和下面的数据来源保持固定
        let footer_reserve = 42.0;
        let mid_h = (full.bottom() - footer_reserve - ui.cursor().top()).max(96.0);
        egui::ScrollArea::vertical()
            .id_salt("sidebar-nav")
            .max_height(mid_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_width(w);
                let items = [
                    (View::Discover, "发现", Icon::Compass, String::new()),
                    (
                        View::National,
                        "央广频率",
                        Icon::Radio,
                        format!("{}", self.national.len().max(19)),
                    ),
                    (
                        View::All,
                        "全部电台",
                        Icon::Broadcast,
                        if self.all.is_empty() {
                            String::new()
                        } else {
                            format!("{}", self.all.len())
                        },
                    ),
                    (
                        View::Favorites,
                        "我的收藏",
                        Icon::Heart,
                        format!("{}", self.store.favorites.len()),
                    ),
                    (View::Recent, "最近播放", Icon::Clock, String::new()),
                ];
                for (view, label, icon, badge) in items {
                    let active = self.view == view;
                    if self.nav_item(ui, label, icon, &badge, active) {
                        self.view = view;
                    }
                }

                // 「按地区」是分组标题：几何和导航项完全对齐（36 高、图标 left+27、
                // 文字 left+45、12.5px），只在右侧放一个展开箭头，看起来才是一组
                ui.add_space(4.0);
                let (head, hresp) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::click());
                {
                    let p = ui.painter().clone();
                    let in_region = self.view == View::Region;
                    if hresp.hovered() {
                        p.rect_filled(
                            head.shrink2(Vec2::new(10.0, 2.0)),
                            CornerRadius::same(8),
                            theme::white_alpha(8),
                        );
                    }
                    let color = if in_region { theme::TEXT } else { theme::DIM };
                    icons::draw(
                        &p,
                        Rect::from_center_size(
                            Pos2::new(head.left() + 27.0, head.center().y),
                            Vec2::splat(16.0),
                        ),
                        Icon::MapPin,
                        color,
                        17.0,
                    );
                    audit::text(
                        &p,
                        Pos2::new(head.left() + 45.0, head.center().y),
                        Align2::LEFT_CENTER,
                        "按地区",
                        theme::regular(12.5),
                        color,
                    );
                    icons::draw(
                        &p,
                        Rect::from_center_size(
                            Pos2::new(head.right() - 25.0, head.center().y),
                            Vec2::splat(14.0),
                        ),
                        if self.prov_open {
                            Icon::ChevronDown
                        } else {
                            Icon::ChevronRight
                        },
                        theme::FAINT,
                        13.0,
                    );
                }
                if hresp.clicked() {
                    self.prov_open = !self.prov_open;
                    self.store.prov_open = self.prov_open;
                    self.save_store();
                }
                if self.prov_open {
                    ui.add_space(2.0);
                    // 一条细导轨，把省份列表和上面的「按地区」连成一组，层级更明确
                    let rail_top = head.bottom() - 6.0;
                    self.province_tree(ui);
                    let rail_bottom = ui.cursor().top() - 4.0;
                    if rail_bottom > rail_top + 4.0 {
                        ui.painter().line_segment(
                            [
                                Pos2::new(head.left() + 27.0, rail_top),
                                Pos2::new(head.left() + 27.0, rail_bottom),
                            ],
                            Stroke::new(1.0, theme::white_alpha(14)),
                        );
                    }
                }

                ui.add_space(12.0);
                audit::text(
                    ui.painter(),
                    Pos2::new(full.left() + 22.0, ui.cursor().top()),
                    Align2::LEFT_TOP,
                    "其 他",
                    theme::regular(9.5),
                    theme::FAINT,
                );
                ui.add_space(16.0);

                if self.nav_item(ui, "设置", Icon::Gear, "", self.view == View::Settings) {
                    self.view = View::Settings;
                }
                ui.add_space(6.0);
            });

        // 底部品牌说明
        let bottom = full.bottom() - 26.0;
        audit::text(
            ui.painter(),
            Pos2::new(full.left() + 20.0, bottom),
            Align2::LEFT_CENTER,
            "数据来源 云听",
            theme::regular(9.5),
            theme::FAINT,
        );
    }

    fn nav_item(
        &mut self,
        ui: &mut egui::Ui,
        label: &str,
        icon: Icon,
        badge: &str,
        active: bool,
    ) -> bool {
        let left = ui.max_rect().left();
        let width = theme::SIDEBAR_W;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 36.0), Sense::click());
        let hovered = resp.hovered();
        if active || hovered {
            ui.painter().rect_filled(
                rect.shrink2(Vec2::new(10.0, 2.0)),
                CornerRadius::same(8),
                if active {
                    theme::accent_alpha(20)
                } else {
                    theme::white_alpha(8)
                },
            );
        }
        if active {
            ui.painter().rect_filled(
                Rect::from_min_size(
                    Pos2::new(left, rect.top() + 9.0),
                    Vec2::new(2.5, rect.height() - 18.0),
                ),
                CornerRadius::same(2),
                theme::ACCENT,
            );
        }
        let color = if active { theme::TEXT } else { theme::DIM };
        icons::draw(
            ui.painter(),
            Rect::from_center_size(
                Pos2::new(rect.left() + 27.0, rect.center().y),
                Vec2::splat(16.0),
            ),
            icon,
            color,
            17.0,
        );
        audit::text(
            ui.painter(),
            Pos2::new(rect.left() + 45.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            if active {
                theme::bold(12.5)
            } else {
                theme::regular(12.5)
            },
            color,
        );
        if !badge.is_empty() {
            audit::text(
                ui.painter(),
                Pos2::new(rect.right() - 18.0, rect.center().y),
                Align2::RIGHT_CENTER,
                badge,
                theme::regular(10.5),
                theme::FAINT,
            );
        }
        resp.clicked()
    }

    fn province_tree(&mut self, ui: &mut egui::Ui) {
        let left = ui.max_rect().left();
        ui.painter().line_segment(
            [
                Pos2::new(left + 27.0, ui.cursor().top()),
                Pos2::new(left + 27.0, ui.cursor().top() + 4.0),
            ],
            Stroke::new(1.0, theme::white_alpha(20)),
        );
        let list = self.provinces.clone();
        let mut picked: Option<u32> = None;
        {
            for p in list.iter() {
                let active = self.view == View::Region && self.prov == p.code;
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(theme::SIDEBAR_W, 28.0), Sense::click());
                if resp.hovered() || active {
                    ui.painter().rect_filled(
                        rect.shrink2(Vec2::new(10.0, 1.0)),
                        CornerRadius::same(7),
                        if active {
                            theme::accent_alpha(24)
                        } else {
                            theme::white_alpha(8)
                        },
                    );
                }
                audit::text(
                    ui.painter(),
                    Pos2::new(rect.left() + 45.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    &p.name,
                    theme::regular(12.0),
                    if active { theme::ACCENT } else { theme::DIM },
                );
                if resp.clicked() {
                    picked = Some(p.code);
                }
            }
        }
        if let Some(code) = picked {
            self.open_province(code);
        }
    }

    fn open_province(&mut self, code: u32) {
        self.prov_open = true;
        self.view = View::Region;
        self.prov = code;
        if !self.by_province.contains_key(&code) && !self.province_loading.contains_key(&code) {
            self.province_loading.insert(code, Instant::now());
            self.spawn(move |c| match c.by_province(code) {
                Ok(list) => Msg::Province(code, list),
                Err(e) => Msg::Fail(e.ui_hint().into()),
            });
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let rect = Rect::from_min_size(
            ui.cursor().min,
            Vec2::new(ui.available_width(), theme::TOOLBAR_H),
        );
        let p = ui.painter().clone();
        p.rect_filled(rect, CornerRadius::ZERO, theme::BG);
        p.line_segment(
            [
                Pos2::new(rect.left(), rect.bottom()),
                Pos2::new(rect.right(), rect.bottom()),
            ],
            Stroke::new(1.0, theme::white_alpha(14)),
        );

        let title = self.view.title();
        // 搜索页自己带输入框，工具条只留标题
        if self.view == View::Search {
            audit::text(
                &p,
                Pos2::new(rect.left() + 22.0, rect.center().y),
                Align2::LEFT_CENTER,
                title,
                theme::bold(15.0),
                theme::TEXT,
            );
            window_drag(ui, rect, "toolbar-drag");
            ui.allocate_space(Vec2::new(ui.available_width(), theme::TOOLBAR_H));
            return;
        }
        let count = match self.view {
            View::National => format!("{} 个频率", self.national.len()),
            View::Region => {
                let n = self
                    .by_province
                    .get(&self.prov)
                    .map(|v| v.len())
                    .unwrap_or(0);
                format!("{} · {n} 个台", self.province_name(self.prov))
            }
            View::Favorites => format!("{} 个台", self.store.favorites.len()),
            View::Recent => format!("{} 条记录", self.store.recents.len()),
            _ => String::new(),
        };

        // 右侧控件组：视图切换 · 刷新 · 节目单开关，装在一个容器里，视觉上成组
        let group_w = 62.0 + 4.0 + 30.0 + 4.0 + 30.0 + 8.0;
        let group = Rect::from_center_size(
            Pos2::new(rect.right() - 14.0 - group_w * 0.5, rect.center().y),
            Vec2::new(group_w, 32.0),
        );
        // 工具条左侧空白 + 标题区可拖动窗口（右侧控件组除外）
        window_drag(
            ui,
            Rect::from_min_max(rect.min, Pos2::new(group.left() - 6.0, rect.bottom())),
            "toolbar-drag",
        );
        p.rect(
            group,
            CornerRadius::same(10),
            Color32::from_rgb(0x0b, 0x0d, 0x10),
            Stroke::new(1.0, theme::white_alpha(16)),
            StrokeKind::Inside,
        );

        let seg = Rect::from_min_size(
            Pos2::new(group.left() + 4.0, group.center().y - 14.0),
            Vec2::new(62.0, 28.0),
        );
        let half = Vec2::new(29.0, 24.0);
        // 注意：两个按钮必须落在各自的半格中心（左 15.5 / 右 15.5），
        // 之前都用了 seg 中心，两个矩形几乎重合，后注册的列表按钮总是抢到点击
        let g_rect = Rect::from_center_size(Pos2::new(seg.left() + 15.5, seg.center().y), half);
        let l_rect = Rect::from_center_size(Pos2::new(seg.right() - 15.5, seg.center().y), half);
        // 只有「电台列表」页面才受这个开关影响；发现页/全部电台/设置页置灰，
        // 否则点下去毫无反馈，看起来像坏了
        let grid_applies = matches!(
            self.view,
            View::National | View::Region | View::Favorites | View::Recent
        );
        let (_g_resp, _l_resp) = if grid_applies {
            let g = {
                let _r = g_rect;
                audit::ctl("tb-grid", _r, ui.clip_rect());
                ui.interact(_r, ui.scope_id().with("tb-grid"), Sense::click())
            };
            let l = {
                let _r = l_rect;
                audit::ctl("tb-list", _r, ui.clip_rect());
                ui.interact(_r, ui.scope_id().with("tb-list"), Sense::click())
            };
            if g.clicked() {
                self.grid_mode = true;
            }
            if l.clicked() {
                self.grid_mode = false;
            }
            (g, l)
        } else {
            (
                {
                    let _r = g_rect;
                    audit::ctl("tb-grid", _r, ui.clip_rect());
                    ui.interact(_r, ui.scope_id().with("tb-grid"), Sense::hover())
                },
                {
                    let _r = l_rect;
                    audit::ctl("tb-list", _r, ui.clip_rect());
                    ui.interact(_r, ui.scope_id().with("tb-list"), Sense::hover())
                },
            )
        };
        let dim = if grid_applies { 1.0 } else { 0.45 };
        if self.grid_mode {
            p.rect_filled(
                g_rect,
                CornerRadius::same(6),
                theme::ELEV2.gamma_multiply(dim),
            );
        } else {
            p.rect_filled(
                l_rect,
                CornerRadius::same(6),
                theme::ELEV2.gamma_multiply(dim),
            );
        }
        icons::draw(
            &p,
            g_rect,
            Icon::Grid,
            if self.grid_mode {
                theme::TEXT
            } else {
                theme::WEAK
            }
            .gamma_multiply(dim),
            15.0,
        );
        icons::draw(
            &p,
            l_rect,
            Icon::List,
            if !self.grid_mode {
                theme::TEXT
            } else {
                theme::WEAK
            }
            .gamma_multiply(dim),
            15.0,
        );

        let refresh_rect = Rect::from_center_size(
            Pos2::new(seg.right() + 4.0 + 15.0, group.center().y),
            Vec2::splat(28.0),
        );
        // 刷新只对能拉取远程数据的页面有效；收藏/最近/设置置灰
        let refresh_applies = matches!(
            self.view,
            View::Discover | View::National | View::All | View::Region
        );
        let rresp = {
            let _r = refresh_rect;
            audit::ctl("tb-refresh", _r, ui.clip_rect());
            ui.interact(
                _r,
                ui.scope_id().with("tb-refresh"),
                if refresh_applies {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            )
        };
        if rresp.clicked() && refresh_applies {
            self.refresh();
        }
        icons::draw(
            &p,
            refresh_rect,
            Icon::Refresh,
            if !refresh_applies {
                theme::FAINT
            } else if rresp.hovered() {
                theme::TEXT
            } else {
                theme::WEAK
            },
            16.0,
        );

        let panel_rect = Rect::from_center_size(
            Pos2::new(refresh_rect.right() + 4.0 + 15.0, group.center().y),
            Vec2::splat(28.0),
        );
        let presp = {
            let _r = panel_rect;
            audit::ctl("tb-panel", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("tb-panel"), Sense::click())
        };
        if presp.clicked() {
            self.panel_open = !self.panel_open;
            self.save_store();
        }
        icons::draw(
            &p,
            panel_rect,
            Icon::Panel,
            if self.panel_open {
                theme::TEXT
            } else if presp.hovered() {
                theme::DIM
            } else {
                theme::WEAK
            },
            16.0,
        );

        // 标题 + 计数
        audit::text(
            &p,
            Pos2::new(rect.left() + 22.0, rect.center().y),
            Align2::LEFT_CENTER,
            title,
            theme::bold(15.0),
            theme::TEXT,
        );
        let tw = p
            .layout_no_wrap(title.to_owned(), theme::bold(15.0), theme::TEXT)
            .size()
            .x;
        if !count.is_empty() {
            audit::text(
                &p,
                Pos2::new(rect.left() + 22.0 + tw + 10.0, rect.center().y + 1.0),
                Align2::LEFT_CENTER,
                &count,
                theme::regular(11.0),
                theme::WEAK,
            );
        }

        // 缓存提示：贴在控件组左侧，低调的琥珀色
        if self.freshness != Freshness::Live && self.freshness.minutes() >= 10 {
            let label = format!("列表缓存 · {} 分钟前", self.freshness.minutes());
            let font = theme::regular(10.0);
            let w = p
                .layout_no_wrap(label.clone(), font.clone(), theme::AMBER)
                .size()
                .x
                + 22.0;
            let badge = Rect::from_min_size(
                Pos2::new(group.left() - 12.0 - w, rect.center().y - 11.0),
                Vec2::new(w, 22.0),
            );
            p.rect(
                badge,
                CornerRadius::same(theme::R_PILL),
                theme::amber_alpha(14),
                Stroke::NONE,
                StrokeKind::Inside,
            );
            audit::text(
                &p,
                badge.center(),
                Align2::CENTER_CENTER,
                label,
                font,
                theme::AMBER,
            );
        }

        ui.allocate_space(Vec2::new(ui.available_width(), theme::TOOLBAR_H));
    }

    fn province_name(&self, code: u32) -> String {
        self.provinces
            .iter()
            .find(|p| p.code == code)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| "全国".into())
    }

    /// 刷新当前页的数据，不改变所在页面
    fn refresh(&mut self) {
        match self.view {
            View::Discover => {
                self.index_loading = true;
                self.spawn(move |c| match c.all_stations(true) {
                    Ok((list, fresh)) => {
                        let index = StationIndex::build(&list);
                        Msg::Index(list, index, fresh)
                    }
                    Err(e) => Msg::Fail(e.ui_hint().into()),
                });
                self.spawn(move |c| match c.national() {
                    Ok(list) => Msg::National(list, Freshness::Live),
                    Err(e) => Msg::Fail(e.ui_hint().into()),
                });
                self.toast("正在刷新电台列表…");
            }
            View::National => {
                self.spawn(move |c| match c.national() {
                    Ok(list) => Msg::National(list, Freshness::Live),
                    Err(e) => Msg::Fail(e.ui_hint().into()),
                });
                self.toast("正在刷新央广频率…");
            }
            View::Region => {
                let code = self.prov;
                self.by_province.remove(&code);
                self.province_loading.insert(code, Instant::now());
                self.spawn(move |c| match c.by_province(code) {
                    Ok(list) => Msg::Province(code, list),
                    Err(e) => Msg::Fail(e.ui_hint().into()),
                });
                self.toast("正在刷新该地区…");
            }
            View::All => {
                self.index_loading = true;
                self.spawn(move |c| match c.all_stations(true) {
                    Ok((list, fresh)) => {
                        let index = StationIndex::build(&list);
                        Msg::Index(list, index, fresh)
                    }
                    Err(e) => Msg::Fail(e.ui_hint().into()),
                });
                self.toast("正在刷新电台列表…");
            }
            // 收藏 / 最近播放 / 设置都是本地数据，服务端没有可刷新的内容
            _ => {}
        }
    }

    /// 从侧栏提交：把关键词带进结果页输入框再搜索（只单向同步，两边各存各的）
    fn submit_nav_search(&mut self) {
        let kw = self.nav_query.trim().to_string();
        if kw.is_empty() {
            return;
        }
        self.search_input = kw;
        self.view = View::Search;
        self.search_focus = false;
        self.run_search();
    }

    /// 搜索页：提交关键词，从第 0 页开始查服务端
    fn run_search(&mut self) {
        let kw = self.search_input.trim().to_string();
        if kw.is_empty() {
            self.query.clear();
            self.results.clear();
            self.search_total = 0;
            self.search_page = 0;
            self.search_error = None;
            self.search_loading = false;
            self.search_seq += 1;
            return;
        }
        self.query = kw.clone();
        self.results.clear();
        self.search_total = 0;
        self.search_page = 0;
        self.search_pages = 0;
        self.search_error = None;
        self.search_loading = true;
        self.search_seq += 1;
        let seq = self.search_seq;
        self.spawn(move |c| match c.search_stations(&kw, 0, SEARCH_PAGE_SIZE) {
            Ok((items, total)) => Msg::SearchDone(seq, kw, 0, items, total),
            Err(e) => Msg::SearchFail(seq, e.ui_hint().into()),
        });
    }

    /// 翻到指定页（pageNo 0 基，每页 40 条）
    fn go_search_page(&mut self, page: u32) {
        if self.search_loading || self.query.is_empty() || page >= self.search_pages.max(1) {
            return;
        }
        self.search_error = None;
        self.search_loading = true;
        self.search_seq += 1;
        let seq = self.search_seq;
        let kw = self.query.clone();
        self.spawn(
            move |c| match c.search_stations(&kw, page, SEARCH_PAGE_SIZE) {
                Ok((items, total)) => Msg::SearchDone(seq, kw, page, items, total),
                Err(e) => Msg::SearchFail(seq, e.ui_hint().into()),
            },
        );
    }
}

/// 服务端条目缺台别/省份/在播节目：按 contentId 用本地目录补全。
fn enrich_station(mut s: Station, index: &StationIndex) -> Station {
    if let Some(local) = index.get(&s.content_id) {
        return local.clone();
    }
    if s.tag_name.is_empty() {
        s.tag_name = "电台".into();
    }
    s
}

// ------------------------------------------------------------------ eframe

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        audit::begin_frame();
        let ctx_owned = root.ctx().clone();
        let ctx = &ctx_owned;
        if self.full_pending {
            let view = root.max_rect();
            root.painter().rect_filled(view, 0.0, theme::BG);
            self.full_pending = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
                1280.0, 800.0,
            )));
            ctx.request_repaint();
            return;
        }
        if self.mini_pending {
            // 先以当前大窗口画一帧迷你底色，下一帧再真正改窗口尺寸。
            // 否则 macOS 会把上一帧的大界面直接裁进 420x132，看起来就是图标突然巨大再闪一下。
            let view = root.max_rect();
            root.painter().rect_filled(view, 0.0, theme::BG);
            let card = egui::Rect::from_min_size(view.min, egui::Vec2::new(420.0, 132.0));
            root.painter().rect_filled(
                card,
                egui::CornerRadius::same(10),
                Color32::from_rgb(0x14, 0x16, 0x1c),
            );
            self.mini_pending = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::Vec2::new(
                420.0, 132.0,
            )));
            ctx.request_repaint();
            return;
        }
        self.audit_tick(ctx);
        self.live_check_tick(ctx);
        self.pending_replay_tick();
        audit::set_view(self.view.title());
        // 全局快捷键：⌘/Ctrl K 聚焦搜索框
        let search_hotkey = ctx.input(|i| i.key_pressed(egui::Key::K) && i.modifiers.command);
        if search_hotkey {
            if self.view == View::Search {
                self.search_focus = true;
            } else {
                self.nav_search_focus = true;
            }
        }
        // 空格播放/暂停（输入框有焦点时不抢）
        let space = ctx.input(|i| i.key_pressed(egui::Key::Space) && !i.modifiers.command);
        if space && !ctx.egui_wants_keyboard_input() {
            self.toggle_play();
        }

        self.poll(ctx);
        self.images.poll(ctx);
        self.tick(ctx);

        if root.max_rect().width() < 760.0 {
            self.mini_ui(root);
            return;
        }

        // 自绘标题栏：macOS 留出系统红绿灯，其它平台在右侧画窗口按钮
        egui::Panel::top("titlebar")
            .exact_size(TITLEBAR_H)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(theme::PANEL))
            .show(root, |ui| self.titlebar(ui));

        egui::Panel::bottom("player")
            .exact_size(theme::PLAYER_H)
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(0x0d, 0x0f, 0x13)))
            .show(root, |ui| self.player_bar(ui));

        if self.panel_open {
            egui::Panel::right("programs")
                .exact_size(theme::PANEL_W)
                .resizable(false)
                .frame(egui::Frame::NONE.fill(theme::PANEL))
                .show(root, |ui| self.program_panel(ui));
        }

        egui::Panel::left("nav")
            .exact_size(theme::SIDEBAR_W)
            .resizable(false)
            .frame(egui::Frame::NONE.fill(theme::PANEL))
            .show(root, |ui| self.sidebar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::BG))
            .show(root, |ui| {
                self.toolbar(ui);
                self.content(ui);
            });

        self.toast_overlay(ctx);

        // 加载中骨架
        if self.index_loading && self.national.is_empty() {
            self.splash(ctx);
        }

        // 无边框窗口自己处理四边/四角的拖拽缩放
        self.resize_edges(root);
    }
}

// ------------------------------------------------------------------ 内容

impl App {
    fn content(&mut self, ui: &mut egui::Ui) {
        let area = ui.available_rect_before_wrap();

        // 全部电台 / 搜索都是「自己滚动」的整页：输入框或表头固定，列表内部滚动
        if self.view == View::All || self.view == View::Search {
            let inner = Rect::from_min_max(
                Pos2::new(area.left() + 22.0, area.top() + 18.0),
                Pos2::new(area.right() - 22.0, area.bottom() - 12.0),
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                if self.view == View::All {
                    self.view_all(ui);
                } else {
                    self.view_search(ui);
                }
            });
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt(format!("scroll-{:?}", self.view))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    ui.vertical(|ui| {
                        ui.set_width(ui.available_width() - 22.0);
                        match self.view {
                            View::Discover => self.view_discover(ui),
                            View::National => self.view_national(ui),
                            View::All => {}
                            View::Region => {
                                if self.prov == 0 {
                                    if let Some(p) = self.provinces.first().cloned() {
                                        self.open_province(p.code);
                                    }
                                }
                                self.view_region(ui)
                            }
                            View::Favorites => {
                                let list = self.store.fav_stations();
                                self.view_stations(ui, "我的收藏", list)
                            }
                            View::Recent => {
                                let list = self.store.recent_stations();
                                self.view_stations(ui, "最近播放", list)
                            }
                            View::Settings => self.view_settings(ui),
                            View::Search => {}
                        }
                    });
                });
                ui.add_space(26.0);
            });
        // 顶部渐隐
        let p = ui.painter().clone();
        widgets::fade_mask(
            &p,
            Rect::from_min_size(area.min, Vec2::new(area.width(), 14.0)),
            false,
            theme::BG,
        );
    }

    fn view_discover(&mut self, ui: &mut egui::Ui) {
        self.hero(ui);
        ui.add_space(18.0);

        let nat = self.national.clone();
        self.section_head(
            ui,
            "央广频率",
            &format!("共 {} 个台 · 点卡片直接收听", nat.len()),
            true,
        );
        self.station_grid(ui, &nat.iter().take(6).cloned().collect::<Vec<_>>());
        ui.add_space(18.0);

        self.section_head(ui, "热门地区", "按台数排序", false);
        // 用全国索引里的 province_code 统计台数，避免「只显示已加载的省」
        let mut counts: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for s in &self.all {
            if s.province_code != 0 {
                *counts.entry(s.province_code).or_insert(0) += 1;
            }
        }
        let mut hot: Vec<(u32, String, usize)> = counts
            .into_iter()
            .filter_map(|(code, n)| {
                self.provinces
                    .iter()
                    .find(|p| p.code == code)
                    .map(|p| (code, p.name.clone(), n))
            })
            .collect();
        hot.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.cmp(&b.1)));
        hot.truncate(7);
        self.chips(ui, hot);
        ui.add_space(24.0);

        self.section_head(ui, "最近播放", "继续上次听的", false);
        let recents = self.store.recent_stations();
        if recents.is_empty() {
            self.empty_hint(
                ui,
                "还没有播放记录",
                "点任意电台开始收听，这里会记住你最近听的 30 个台。",
            );
        } else {
            self.recent_tiles(ui, &recents[..recents.len().min(4)]);
        }
    }

    fn view_national(&mut self, ui: &mut egui::Ui) {
        let nat = self.national.clone();
        ui.label(RichText::new("中央广播电视总台").size(16.0).strong());
        ui.add_space(2.0);
        ui.label(
            RichText::new(format!(
                "{} 个频率 · {}",
                nat.len(),
                if self.grid_mode {
                    "点卡片直接收听"
                } else {
                    "点一行直接收听"
                }
            ))
            .size(11.5)
            .color(theme::WEAK),
        );
        ui.add_space(14.0);
        if self.grid_mode {
            self.station_grid(ui, &nat);
        } else {
            self.station_rows(ui, &nat);
        }
    }

    fn view_region(&mut self, ui: &mut egui::Ui) {
        let code = self.prov;
        let list = self.by_province.get(&code).cloned().unwrap_or_default();
        let name = self.province_name(code);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&name).size(16.0).strong());
            ui.label(
                RichText::new(format!("{} 个电台", list.len()))
                    .size(11.5)
                    .color(theme::WEAK),
            );
        });
        ui.add_space(14.0);
        if list.is_empty() {
            self.empty_hint(ui, "正在获取电台…", "每个省一次请求，首次打开约 300ms。");
        } else if self.grid_mode {
            self.station_grid(ui, &list);
        } else {
            self.station_rows(ui, &list);
        }
    }

    fn view_stations(&mut self, ui: &mut egui::Ui, title: &str, list: Vec<Station>) {
        ui.label(RichText::new(title).size(16.0).strong());
        ui.add_space(2.0);
        ui.label(
            RichText::new(format!("{} 个台", list.len()))
                .size(11.5)
                .color(theme::WEAK),
        );
        ui.add_space(14.0);
        if list.is_empty() {
            let hint = if title.contains("收藏") {
                (
                    "还没有收藏的电台",
                    "在卡片或播放条上点心形即可收藏，收藏只记台号，播放时才取地址。",
                )
            } else {
                ("最近播放是空的", "听过任何一个台后会出现在这里。")
            };
            self.empty_hint(ui, hint.0, hint.1);
        } else if self.grid_mode {
            self.station_grid(ui, &list);
        } else {
            self.station_rows(ui, &list);
        }
    }

    fn view_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("设置").size(16.0).strong());
        ui.add_space(16.0);
        let (bytes, files) = self.client.cache().stats();
        ui.label(RichText::new("播放").size(12.0).color(theme::WEAK));
        ui.add_space(6.0);
        egui::Frame::NONE
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(theme::R_CARD))
            .stroke(Stroke::new(1.0, theme::white_alpha(16)))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("默认音质").size(12.5));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut high = self.store.quality_high;
                        if ui.selectable_label(!high, "标清").clicked() && high {
                            high = false;
                        }
                        if ui.selectable_label(high, "高清 AAC").clicked() && !high {
                            high = true;
                        }
                        if high != self.store.quality_high {
                            self.store.quality_high = high;
                            self.save_store();
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new("切换后，下一次播放生效。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
        ui.add_space(16.0);

        ui.label(RichText::new("存储").size(12.0).color(theme::WEAK));
        ui.add_space(6.0);
        egui::Frame::NONE
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(theme::R_CARD))
            .stroke(Stroke::new(1.0, theme::white_alpha(16)))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "缓存 {:.1} MB · {} 个文件",
                            bytes as f64 / 1_048_576.0,
                            files
                        ))
                        .size(12.5),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button(RichText::new("清理台标缓存").size(11.5))
                            .clicked()
                        {
                            let _ = self.client.cache().clear_sub("images");
                        }
                    });
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("数据目录 {}", self.data_dir.display()))
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
        ui.add_space(16.0);

        ui.label(RichText::new("关于").size(12.0).color(theme::WEAK));
        ui.add_space(6.0);
        egui::Frame::NONE
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(theme::R_CARD))
            .stroke(Stroke::new(1.0, theme::white_alpha(16)))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("云听电台").size(12.5).strong());
                    ui.label(RichText::new(env!("CARGO_PKG_VERSION")).size(11.0).color(theme::DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let font = theme::regular(9.5);
                        let w = ui.painter().layout_no_wrap("非官方客户端".to_string(), font.clone(), theme::AMBER).size().x + 16.0;
                        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 18.0), Sense::hover());
                        ui.painter().rect(
                            rect,
                            CornerRadius::same(theme::R_PILL),
                            theme::amber_alpha(18),
                            Stroke::new(1.0, theme::amber_alpha(70)),
                            StrokeKind::Inside,
                        );
                        audit::text(ui.painter(), rect.center(), Align2::CENTER_CENTER, "非官方客户端", font, theme::AMBER);
                    });
                });
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("解码器 {}", rust_radio_audio::decoder_name()))
                        .size(10.5)
                        .color(theme::FAINT),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "本程序是非官方的第三方客户端，与中央广播电视总台、云听无任何关联，仅供学习与个人使用。\
                         所有音频、台标与节目内容版权归原权利人所有；本程序不内置、不缓存、不分发任何音频，\
                         请勿用于商业用途。",
                    )
                    .size(10.5)
                    .color(theme::WEAK),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("接口为公开数据接口，若官方调整可能失效。")
                        .size(10.5)
                        .color(theme::FAINT),
                );
            });
    }
}
// ------------------------------------------------------------------ 组件

impl App {
    fn toggle_play(&mut self) {
        let snap = self.engine.snapshot();
        let Some(now) = self.now.as_ref() else {
            if let Some(id) = self.selected.clone() {
                self.select_station(&id, true);
            }
            return;
        };
        let is_replay = now.is_replay;
        let id = now.station.content_id.clone();
        let program_id = now.program_id.clone();
        match snap.state {
            PlayState::Playing | PlayState::Buffering => {
                if is_replay {
                    self.engine.set_paused(true);
                } else {
                    self.engine.stop();
                }
            }
            PlayState::Paused => self.engine.set_paused(false),
            _ => {
                if let Some(pid) = program_id {
                    if let Some(p) = self.programs.iter().find(|p| p.id == pid).cloned() {
                        self.play_program(p);
                    }
                } else {
                    self.select_station(&id, true);
                }
            }
        }
    }

    fn hero(&mut self, ui: &mut egui::Ui) {
        let station = self
            .now
            .as_ref()
            .map(|n| n.station.clone())
            .or_else(|| self.selected.as_ref().and_then(|id| self.station_by_id(id)))
            .or_else(|| self.national.first().cloned());
        let Some(station) = station else { return };
        let detail = self.detail.clone().filter(|d| d.id == station.content_id);
        let bg_url = detail
            .as_ref()
            .and_then(|d| d.play_background_img.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| station.image.clone());

        let h = 168.0;
        let (rect, _resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
        let p = ui.painter().clone();
        let radius = CornerRadius::same(16);

        p.rect_filled(rect, radius, theme::ELEV);

        // 背景：模糊图 + 双渐变
        if let Some(tex) = self.images.blurred(ui.ctx(), &bg_url) {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            child.add(
                egui::Image::new((tex.id(), rect.size()))
                    .corner_radius(radius)
                    .tint(Color32::from_white_alpha(150))
                    .maintain_aspect_ratio(false),
            );
        } else if let Some(tex) = self.images.get(ui.ctx(), &station.image) {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
            child.add(
                egui::Image::new((tex.id(), rect.size()))
                    .corner_radius(radius)
                    .tint(Color32::from_white_alpha(70))
                    .maintain_aspect_ratio(false),
            );
        }
        widgets::gradient_h(
            &p,
            rect,
            Color32::from_rgba_unmultiplied(8, 9, 11, 246),
            Color32::from_rgba_unmultiplied(8, 9, 11, 40),
        );
        widgets::gradient_v(
            &p,
            Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.center().y)),
            Color32::TRANSPARENT,
            Color32::from_rgba_unmultiplied(8, 9, 11, 120),
        );

        // 文案
        let t = self.start.elapsed().as_secs_f32();
        let dot = Pos2::new(rect.left() + 24.0, rect.top() + 30.0);
        widgets::live_dot(&p, dot, 2.6, t, theme::ACCENT);
        audit::text(
            &p,
            Pos2::new(dot.x + 12.0, dot.y),
            Align2::LEFT_CENTER,
            "此刻正在播",
            theme::bold(10.0),
            theme::ACCENT,
        );
        let title_rect = audit::text(
            &p,
            Pos2::new(rect.left() + 24.0, rect.top() + 46.0),
            Align2::LEFT_TOP,
            &station.title,
            theme::bold(30.0),
            theme::TEXT,
        );
        let meta = format!(
            "{} · {} · 高清 AAC",
            station.now_playing(),
            if station.tag_name.is_empty() {
                "中央"
            } else {
                &station.tag_name
            }
        );
        audit::text(
            &p,
            Pos2::new(rect.left() + 24.0, title_rect.bottom() + 1.0),
            Align2::LEFT_TOP,
            meta,
            theme::regular(12.0),
            theme::DIM,
        );

        // 播放按钮
        let play_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 24.0, rect.bottom() - 50.0),
            Vec2::new(112.0, 36.0),
        );
        let snap = self.engine.snapshot();
        let playing_this = self
            .now
            .as_ref()
            .map(|n| n.station.content_id == station.content_id && snap.state.is_on())
            .unwrap_or(false);
        let presp = {
            let _r = play_rect;
            audit::ctl("hero-play", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("hero-play"), Sense::click())
        };
        p.rect_filled(
            play_rect,
            CornerRadius::same(theme::R_PILL),
            if presp.hovered() {
                theme::ACCENT_HOVER
            } else {
                theme::ACCENT
            },
        );
        icons::draw(
            &p,
            Rect::from_center_size(
                Pos2::new(play_rect.left() + 24.0, play_rect.center().y),
                Vec2::splat(13.0),
            ),
            if playing_this && snap.state == PlayState::Playing {
                Icon::Pause
            } else {
                Icon::Play
            },
            theme::ON_ACCENT,
            13.0,
        );
        audit::text(
            &p,
            Pos2::new(play_rect.left() + 40.0, play_rect.center().y),
            Align2::LEFT_CENTER,
            if playing_this {
                "正在收听"
            } else {
                "收听"
            },
            theme::bold(12.5),
            theme::ON_ACCENT,
        );
        if presp.clicked() {
            if playing_this {
                self.toggle_play();
            } else {
                let id = station.content_id.clone();
                self.select_station(&id, true);
            }
        }

        // 收藏
        let fav_rect = Rect::from_min_size(
            Pos2::new(play_rect.right() + 10.0, play_rect.top()),
            Vec2::new(36.0, 36.0),
        );
        let fresp = {
            let _r = fav_rect;
            audit::ctl("hero-fav", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("hero-fav"), Sense::click())
        };
        let faved = self.store.is_fav(&station.content_id);
        p.circle_filled(
            fav_rect.center(),
            18.0,
            if fresp.hovered() {
                theme::white_alpha(24)
            } else {
                theme::white_alpha(12)
            },
        );
        p.circle_stroke(
            fav_rect.center(),
            18.0,
            Stroke::new(1.0, theme::white_alpha(34)),
        );
        icons::draw(
            &p,
            fav_rect,
            if faved { Icon::HeartFill } else { Icon::Heart },
            if faved { theme::ACCENT } else { theme::TEXT },
            18.0,
        );
        if fresp.clicked() {
            let now_fav = self.store.toggle_fav(&station);
            self.save_store();
            self.toast(if now_fav {
                "已加入收藏"
            } else {
                "已取消收藏"
            });
        }

        // 接下来
        let next_rect = Rect::from_min_size(
            Pos2::new(rect.right() - 252.0, rect.top() + 18.0),
            Vec2::new(232.0, 0.0),
        );
        let upcoming: Vec<Program> = if self.program_day != 0 {
            Vec::new()
        } else {
            self.programs
                .iter()
                .filter(|p| p.start_time > chrono::Local::now().timestamp_millis())
                .take(3)
                .cloned()
                .collect()
        };
        if !upcoming.is_empty() {
            let h = 22.0 + upcoming.len() as f32 * 19.0 + 10.0;
            let card = Rect::from_min_size(next_rect.min, Vec2::new(232.0, h));
            p.rect(
                card,
                CornerRadius::same(10),
                Color32::from_rgba_unmultiplied(12, 13, 16, 168),
                Stroke::new(1.0, theme::white_alpha(26)),
                StrokeKind::Inside,
            );
            audit::text(
                &p,
                Pos2::new(card.left() + 12.0, card.top() + 12.0),
                Align2::LEFT_TOP,
                "接下来",
                theme::regular(9.5),
                theme::DIM,
            );
            for (i, prog) in upcoming.iter().enumerate() {
                let y = card.top() + 30.0 + i as f32 * 19.0;
                audit::text(
                    &p,
                    Pos2::new(card.left() + 12.0, y),
                    Align2::LEFT_CENTER,
                    widgets::fmt_clock(&prog.start_local()),
                    theme::regular(10.0),
                    theme::WEAK,
                );
                let name =
                    widgets::ellipsize(ui.ctx(), &prog.program_name, &theme::regular(11.0), 150.0);
                audit::text(
                    &p,
                    Pos2::new(card.left() + 50.0, y),
                    Align2::LEFT_CENTER,
                    name,
                    theme::regular(11.0),
                    theme::DIM,
                );
            }
        }

        // 换一个
        let swap = Rect::from_min_size(
            Pos2::new(rect.right() - 98.0, rect.bottom() - 44.0),
            Vec2::new(78.0, 28.0),
        );
        let sresp = {
            let _r = swap;
            audit::ctl("hero-swap", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("hero-swap"), Sense::click())
        };
        p.rect(
            swap,
            CornerRadius::same(theme::R_PILL),
            Color32::from_rgba_unmultiplied(10, 11, 13, 150),
            Stroke::new(1.0, theme::white_alpha(26)),
            StrokeKind::Inside,
        );
        icons::draw(
            &p,
            Rect::from_center_size(
                Pos2::new(swap.left() + 16.0, swap.center().y),
                Vec2::splat(11.0),
            ),
            Icon::Refresh,
            theme::DIM,
            12.0,
        );
        audit::text(
            &p,
            Pos2::new(swap.left() + 26.0, swap.center().y),
            Align2::LEFT_CENTER,
            "换一个",
            theme::regular(10.5),
            theme::DIM,
        );
        if sresp.clicked() && !self.national.is_empty() {
            let idx = (self.start.elapsed().as_millis() as usize / 7) % self.national.len();
            let id = self.national[idx].content_id.clone();
            self.select_station(&id, true);
        }
    }

    fn section_head(&mut self, ui: &mut egui::Ui, title: &str, sub: &str, more: bool) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).size(15.0).strong());
            ui.add_space(2.0);
            ui.label(RichText::new(sub).size(11.0).color(theme::WEAK));
            if more {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let resp = ui.add(
                        egui::Label::new(RichText::new("查看全部 →").size(11.0).color(theme::DIM))
                            .sense(Sense::click()),
                    );
                    if resp.clicked() {
                        self.view = View::National;
                    }
                });
            }
        });
        ui.add_space(8.0);
    }

    fn station_grid(&mut self, ui: &mut egui::Ui, list: &[Station]) {
        if list.is_empty() {
            return;
        }
        let avail = ui.available_width();
        let gap = 13.0;
        let cols = (((avail + gap) / 104.0).floor() as usize).clamp(2, 8);
        let card_w = (avail - gap * (cols.saturating_sub(1)) as f32) / cols as f32;
        let art = card_w - 20.0;
        let card_h = art + 52.0;
        let mut clicked: Option<String> = None;

        for row in list.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for s in row {
                    if self.station_card(ui, s, card_w, card_h, art) {
                        clicked = Some(s.content_id.clone());
                    }
                }
            });
            ui.add_space(gap);
        }
        if let Some(id) = clicked {
            self.activate_station(&id);
        }
    }

    fn station_card(&mut self, ui: &mut egui::Ui, s: &Station, w: f32, h: f32, art: f32) -> bool {
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
        if !ui.is_rect_visible(rect) {
            return false;
        }
        let hovered = resp.hovered();
        let lift = ui.ctx().animate_bool_with_time(
            egui::Id::unique(("card", &s.content_id)),
            hovered,
            0.15,
        );
        let rect = rect.translate(Vec2::new(0.0, -2.0 * lift));
        let p = ui.painter().clone();
        let playing = self
            .now
            .as_ref()
            .map(|n| n.station.content_id == s.content_id)
            .unwrap_or(false);

        p.rect(
            rect,
            CornerRadius::same(theme::R_CARD),
            if hovered {
                Color32::from_rgb(0x16, 0x1a, 0x20)
            } else {
                theme::PANEL
            },
            Stroke::new(
                1.0,
                if playing {
                    theme::accent_alpha(110)
                } else if hovered {
                    theme::white_alpha(30)
                } else {
                    theme::white_alpha(16)
                },
            ),
            StrokeKind::Inside,
        );

        let art_rect = Rect::from_min_size(
            Pos2::new(rect.left() + 10.0, rect.top() + 10.0),
            Vec2::splat(art),
        );
        let tex = self.images.get(ui.ctx(), &s.image);
        widgets::cover(ui, art_rect, theme::R_ART, tex, &s.title);

        // hover 播放圆钮
        if lift > 0.05 {
            let btn = Rect::from_center_size(
                Pos2::new(art_rect.right() - 18.0, art_rect.bottom() - 18.0),
                Vec2::splat(32.0),
            );
            p.circle_filled(btn.center(), 16.0, theme::ACCENT.gamma_multiply(lift));
            icons::draw(
                &p,
                btn,
                if playing && self.engine.snapshot().state == PlayState::Playing {
                    Icon::Pause
                } else {
                    Icon::Play
                },
                theme::ON_ACCENT.gamma_multiply(lift),
                14.0,
            );
        }

        let title_font = theme::bold(13.0);
        let title = widgets::ellipsize(ui.ctx(), &s.title, &title_font, art - 2.0);
        audit::text(
            &p,
            Pos2::new(art_rect.left(), art_rect.bottom() + 12.0),
            Align2::LEFT_CENTER,
            title,
            title_font,
            theme::TEXT,
        );
        let sub_font = theme::regular(11.0);
        let sub = widgets::ellipsize(ui.ctx(), s.now_playing(), &sub_font, art - 2.0);
        audit::text(
            &p,
            Pos2::new(art_rect.left(), art_rect.bottom() + 28.0),
            Align2::LEFT_CENTER,
            sub,
            sub_font,
            theme::WEAK,
        );

        if playing {
            let t = self.start.elapsed().as_secs_f32();
            let bars = Rect::from_min_size(
                Pos2::new(rect.right() - 34.0, rect.top() + 14.0),
                Vec2::new(18.0, 12.0),
            );
            let level = self.engine.snapshot().level;
            widgets::eq_bars(&p, bars, level, t, theme::ACCENT);
        }

        if resp.clicked() {
            return true;
        }
        false
    }

    fn station_row_ui(&mut self, ui: &mut egui::Ui, s: &Station, index: usize) -> bool {
        let (rect, resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 52.0), Sense::click());
        let p = ui.painter().clone();
        let playing = self
            .now
            .as_ref()
            .map(|n| n.station.content_id == s.content_id)
            .unwrap_or(false);
        if playing {
            p.rect_filled(
                rect.shrink2(Vec2::new(2.0, 1.0)),
                CornerRadius::same(8),
                theme::accent_alpha(18),
            );
        } else if resp.hovered() {
            p.rect_filled(
                rect.shrink2(Vec2::new(2.0, 1.0)),
                CornerRadius::same(8),
                theme::white_alpha(9),
            );
        }
        audit::text(
            &p,
            Pos2::new(rect.left() + 16.0, rect.center().y),
            Align2::LEFT_CENTER,
            format!("{:02}", index + 1),
            theme::regular(10.5),
            theme::FAINT,
        );
        let cover_rect = Rect::from_center_size(
            Pos2::new(rect.left() + 52.0, rect.center().y),
            Vec2::splat(36.0),
        );
        let tex = self.images.get(ui.ctx(), &s.image);
        widgets::cover(ui, cover_rect, 8, tex, &s.title);

        let name_font = theme::bold(12.5);
        let name = widgets::ellipsize(ui.ctx(), &s.title, &name_font, 240.0);
        audit::text(
            &p,
            Pos2::new(cover_rect.right() + 12.0, rect.center().y - 8.0),
            Align2::LEFT_CENTER,
            name,
            name_font,
            theme::TEXT,
        );
        let sub_font = theme::regular(10.5);
        let sub = widgets::ellipsize(ui.ctx(), s.now_playing(), &sub_font, 300.0);
        audit::text(
            &p,
            Pos2::new(cover_rect.right() + 12.0, rect.center().y + 9.0),
            Align2::LEFT_CENTER,
            sub,
            sub_font,
            theme::WEAK,
        );

        let tag_w = 74.0;
        let tag_rect = Rect::from_center_size(
            Pos2::new(rect.right() - 170.0, rect.center().y),
            Vec2::new(tag_w, 20.0),
        );
        p.rect(
            tag_rect,
            CornerRadius::same(theme::R_PILL),
            theme::ELEV2,
            Stroke::new(1.0, theme::white_alpha(16)),
            StrokeKind::Inside,
        );
        let tag = widgets::ellipsize(ui.ctx(), &s.tag_name, &theme::regular(9.5), tag_w - 12.0);
        audit::text(
            &p,
            tag_rect.center(),
            Align2::CENTER_CENTER,
            tag,
            theme::regular(9.5),
            theme::DIM,
        );

        // 正在播的那一行常显按钮，其它行悬停才显示；图标随播放状态切换
        if playing || resp.hovered() {
            let btn = Rect::from_center_size(
                Pos2::new(rect.right() - 34.0, rect.center().y),
                Vec2::splat(28.0),
            );
            let is_playing = playing && self.engine.snapshot().state == PlayState::Playing;
            p.circle_filled(btn.center(), 14.0, theme::ACCENT);
            icons::draw(
                &p,
                btn,
                if is_playing { Icon::Pause } else { Icon::Play },
                theme::ON_ACCENT,
                14.0,
            );
        }
        resp.clicked()
    }

    fn station_rows(&mut self, ui: &mut egui::Ui, list: &[Station]) {
        let mut clicked: Option<String> = None;
        egui::Frame::NONE
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(theme::R_CARD))
            .stroke(Stroke::new(1.0, theme::white_alpha(16)))
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                for (i, s) in list.iter().enumerate() {
                    if self.station_row_ui(ui, s, i) {
                        clicked = Some(s.content_id.clone());
                    }
                }
            });
        if let Some(id) = clicked {
            self.activate_station(&id);
        }
    }

    /// 全部 941 个台：只画可见行，避免为上千个台标发请求
    fn view_all(&mut self, ui: &mut egui::Ui) {
        let total = self.all.len();
        ui.horizontal(|ui| {
            ui.label(RichText::new("全部电台").size(16.0).strong());
            ui.label(
                RichText::new(format!("{total} 个台 · 央广 + 31 个省级列表"))
                    .size(11.5)
                    .color(theme::WEAK),
            );
        });
        ui.add_space(12.0);

        // 列表高度 = 视口高度 - 顶部留白 - 标题 - 卡片内边距 - 底部留白，
        // 这样它和「央广频率」「按地区」一样撑满内容区，而不是缩成一行
        let list_h = (ui.available_height() - 8.0).max(240.0);
        let mut clicked: Option<String> = None;
        let row_h = 52.0;
        egui::Frame::NONE
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(theme::R_CARD))
            .stroke(Stroke::new(1.0, theme::white_alpha(16)))
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("all-stations")
                    .auto_shrink([false, false])
                    .max_height(list_h)
                    .show_rows(ui, row_h, total, |ui, range| {
                        for i in range {
                            let s = self.all[i].clone();
                            if self.station_row_ui(ui, &s, i) {
                                clicked = Some(s.content_id);
                            }
                        }
                    });
            });
        if let Some(id) = clicked {
            self.activate_station(&id);
        }
    }

    /// 搜索结果页：顶部输入框 + 搜索按钮，下面是服务端返回的电台列表
    fn view_search(&mut self, ui: &mut egui::Ui) {
        if self.search_loading {
            // 等响应期间保持出帧，否则回包要等下一次输入才会被 poll 到
            ui.ctx().request_repaint_after(Duration::from_millis(120));
        }
        let row_h = 38.0;
        let (row_rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
        let btn_w = 84.0;
        let field_rect = Rect::from_min_size(
            row_rect.min,
            Vec2::new(row_rect.width() - btn_w - 12.0, row_h),
        );
        let btn_rect = Rect::from_min_size(
            Pos2::new(field_rect.right() + 12.0, row_rect.top()),
            Vec2::new(btn_w, row_h),
        );
        ui.painter().rect(
            field_rect,
            CornerRadius::same(theme::R_BTN),
            Color32::from_rgb(0x07, 0x08, 0x0a),
            Stroke::new(1.0, theme::white_alpha(28)),
            StrokeKind::Inside,
        );
        icons::draw(
            ui.painter(),
            Rect::from_center_size(
                Pos2::new(field_rect.left() + 17.0, field_rect.center().y),
                Vec2::splat(14.0),
            ),
            Icon::Search,
            theme::WEAK,
            15.0,
        );
        let inner = Rect::from_min_max(
            Pos2::new(field_rect.left() + 34.0, field_rect.top() + 4.0),
            Pos2::new(field_rect.right() - 10.0, field_rect.bottom() - 4.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let te = egui::TextEdit::singleline(&mut self.search_input)
            .frame(egui::Frame::NONE)
            .hint_text(
                RichText::new("搜索电台，支持台名与拼音（如 zgzs）")
                    .size(13.0)
                    .color(theme::WEAK),
            )
            .text_color(theme::TEXT)
            .desired_width(inner.width())
            .margin(egui::Margin::ZERO)
            .align(Align2::LEFT_CENTER)
            .id(egui::Id::unique("search-page-field"));
        let out = child.add_sized([inner.width(), inner.height()], te);
        if self.search_focus {
            out.request_focus();
            self.search_focus = false;
        }
        let submitted = out.lost_focus() && child.input(|i| i.key_pressed(egui::Key::Enter));
        let btn = egui::Button::new(
            RichText::new("搜索")
                .size(15.0)
                .strong()
                .color(theme::ON_ACCENT),
        )
        .fill(theme::ACCENT)
        .corner_radius(CornerRadius::same(theme::R_BTN));
        if ui.put(btn_rect, btn).clicked() || submitted {
            self.run_search();
        }
        ui.add_space(12.0);

        let status = if self.search_loading {
            "搜索中…".to_string()
        } else if let Some(err) = &self.search_error {
            err.clone()
        } else if self.query.is_empty() {
            "输入关键词后按 Enter 或点「搜索」".to_string()
        } else if self.search_pages > 1 {
            format!(
                "找到 {} 个电台 · 第 {} / {} 页",
                self.search_total,
                self.search_page + 1,
                self.search_pages
            )
        } else {
            format!("找到 {} 个电台", self.search_total)
        };
        let color = if self.search_error.is_some() {
            theme::AMBER
        } else {
            theme::WEAK
        };
        ui.label(RichText::new(status).size(11.0).color(color));
        ui.add_space(10.0);

        if self.results.is_empty() {
            if self.query.is_empty() {
                ui.label(
                    RichText::new("搜台名、台别或拼音，例如「中国之声」「交通」「zgzs」")
                        .size(12.0)
                        .color(theme::FAINT),
                );
            } else if !self.search_loading && self.search_error.is_none() {
                ui.label(
                    RichText::new(format!("没有找到「{}」，换个关键词试试", self.query))
                        .size(12.0)
                        .color(theme::FAINT),
                );
            }
            return;
        }

        let pager_h = if self.search_pages > 1 { 46.0 } else { 0.0 };
        let list_h = (ui.available_height() - 8.0 - pager_h).max(200.0);
        let list = self.results.clone();
        let mut clicked: Option<String> = None;
        egui::Frame::NONE
            .fill(theme::PANEL)
            .corner_radius(CornerRadius::same(theme::R_CARD))
            .stroke(Stroke::new(1.0, theme::white_alpha(16)))
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(format!("search-results-{}", self.search_page))
                    .auto_shrink([false, false])
                    .max_height(list_h)
                    .show(ui, |ui| {
                        for (i, s) in list.iter().enumerate() {
                            if self.station_row_ui(ui, s, i) {
                                clicked = Some(s.content_id.clone());
                            }
                        }
                        ui.add_space(6.0);
                    });
            });
        if let Some(id) = clicked {
            self.activate_station(&id);
        }

        // 分页：首页 / 上一页 / 第 x / y 页 / 下一页 / 尾页（手绘，整组居中）
        if self.search_pages > 1 {
            ui.add_space(8.0);
            let (rect, _) =
                ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
            let btn_w = 64.0;
            let gap = 8.0;
            let label_w = 100.0;
            let total_w = btn_w * 4.0 + gap * 3.0 + label_w;
            let x0 = rect.center().x - total_w * 0.5;
            let first_rect =
                Rect::from_min_size(Pos2::new(x0, rect.top() + 1.0), Vec2::new(btn_w, 28.0));
            let prev_rect = Rect::from_min_size(
                Pos2::new(first_rect.right() + gap, rect.top() + 1.0),
                Vec2::new(btn_w, 28.0),
            );
            let next_rect = Rect::from_min_size(
                Pos2::new(prev_rect.right() + gap + label_w + gap, rect.top() + 1.0),
                Vec2::new(btn_w, 28.0),
            );
            let last_rect = Rect::from_min_size(
                Pos2::new(next_rect.right() + gap, rect.top() + 1.0),
                Vec2::new(btn_w, 28.0),
            );
            let can_prev = self.search_page > 0 && !self.search_loading;
            let can_next = self.search_page + 1 < self.search_pages && !self.search_loading;
            let mut hits: [bool; 4] = [false; 4];
            for (i, (r, name, on)) in [
                (first_rect, "search-first", can_prev),
                (prev_rect, "search-prev", can_prev),
                (next_rect, "search-next", can_next),
                (last_rect, "search-last", can_next),
            ]
            .into_iter()
            .enumerate()
            {
                let resp = {
                    let _r = r;
                    audit::ctl(name, _r, ui.clip_rect());
                    ui.interact(
                        _r,
                        ui.scope_id().with(name),
                        if on { Sense::click() } else { Sense::hover() },
                    )
                };
                hits[i] = resp.clicked() && on;
                let label = ["首页", "上一页", "下一页", "尾页"][i];
                ui.painter().rect(
                    r,
                    CornerRadius::same(theme::R_BTN),
                    theme::ELEV2.gamma_multiply(if on { 1.0 } else { 0.5 }),
                    Stroke::new(1.0, theme::white_alpha(if on { 20 } else { 10 })),
                    StrokeKind::Inside,
                );
                audit::text(
                    ui.painter(),
                    r.center(),
                    Align2::CENTER_CENTER,
                    label,
                    theme::regular(12.0),
                    if on { theme::DIM } else { theme::FAINT },
                );
            }
            audit::text(
                ui.painter(),
                Pos2::new(
                    (prev_rect.right() + next_rect.left()) * 0.5,
                    rect.center().y,
                ),
                Align2::CENTER_CENTER,
                format!("第 {} / {} 页", self.search_page + 1, self.search_pages),
                theme::regular(11.5),
                theme::DIM,
            );
            if hits[0] {
                self.go_search_page(0);
            } else if hits[1] {
                self.go_search_page(self.search_page - 1);
            } else if hits[2] {
                self.go_search_page(self.search_page + 1);
            } else if hits[3] {
                self.go_search_page(self.search_pages - 1);
            }
        }
    }

    fn chips(&mut self, ui: &mut egui::Ui, items: Vec<(u32, String, usize)>) {
        let mut pick: Option<u32> = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
            for (code, name, n) in items {
                let label = format!("{name} {n}");
                let font = theme::regular(11.0);
                let w = ui
                    .painter()
                    .layout_no_wrap(label.clone(), font.clone(), theme::DIM)
                    .size()
                    .x
                    + 26.0;
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::click());
                let active = self.view == View::Region && self.prov == code;
                let p = ui.painter().clone();
                p.rect(
                    rect,
                    CornerRadius::same(theme::R_PILL),
                    if active {
                        theme::accent_alpha(26)
                    } else if resp.hovered() {
                        theme::ELEV2
                    } else {
                        theme::PANEL
                    },
                    Stroke::new(
                        1.0,
                        if active {
                            theme::accent_alpha(110)
                        } else {
                            theme::white_alpha(18)
                        },
                    ),
                    StrokeKind::Inside,
                );
                audit::text(
                    &p,
                    rect.center(),
                    Align2::CENTER_CENTER,
                    label,
                    font,
                    if active { theme::ACCENT } else { theme::DIM },
                );
                if resp.clicked() {
                    pick = Some(code);
                }
            }
        });
        if let Some(code) = pick {
            self.open_province(code);
        }
    }

    fn recent_tiles(&mut self, ui: &mut egui::Ui, list: &[Station]) {
        let mut clicked: Option<String> = None;
        let avail = ui.available_width();
        let gap = 12.0;
        let cols = (((avail + gap) / 190.0).floor() as usize).clamp(2, 5);
        let w = (avail - gap * (cols - 1) as f32) / cols as f32;
        for row in list.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for s in row {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 58.0), Sense::click());
                    let p = ui.painter().clone();
                    let playing = self
                        .now
                        .as_ref()
                        .map(|n| n.station.content_id == s.content_id)
                        .unwrap_or(false);
                    p.rect(
                        rect,
                        CornerRadius::same(10),
                        if playing {
                            theme::accent_alpha(14)
                        } else if resp.hovered() {
                            Color32::from_rgb(0x16, 0x1a, 0x20)
                        } else {
                            theme::PANEL
                        },
                        Stroke::new(
                            1.0,
                            if playing {
                                theme::accent_alpha(80)
                            } else {
                                theme::white_alpha(16)
                            },
                        ),
                        StrokeKind::Inside,
                    );
                    let cover_rect = Rect::from_center_size(
                        Pos2::new(rect.left() + 30.0, rect.center().y),
                        Vec2::splat(40.0),
                    );
                    let tex = self.images.get(ui.ctx(), &s.image);
                    widgets::cover(ui, cover_rect, 8, tex, &s.title);
                    let font = theme::bold(12.0);
                    let name = widgets::ellipsize(ui.ctx(), &s.title, &font, w - 96.0);
                    audit::text(
                        &p,
                        Pos2::new(cover_rect.right() + 11.0, rect.center().y - 8.0),
                        Align2::LEFT_CENTER,
                        name,
                        font,
                        theme::TEXT,
                    );
                    let sub_font = theme::regular(10.0);
                    let sub = widgets::ellipsize(ui.ctx(), &s.subtitle, &sub_font, w - 96.0);
                    audit::text(
                        &p,
                        Pos2::new(cover_rect.right() + 11.0, rect.center().y + 8.0),
                        Align2::LEFT_CENTER,
                        if playing {
                            "正在收听".to_string()
                        } else {
                            sub
                        },
                        sub_font,
                        if playing { theme::ACCENT } else { theme::WEAK },
                    );
                    icons::draw(
                        &p,
                        Rect::from_center_size(
                            Pos2::new(rect.right() - 20.0, rect.center().y),
                            Vec2::splat(12.0),
                        ),
                        Icon::Play,
                        if resp.hovered() {
                            theme::TEXT
                        } else {
                            theme::FAINT
                        },
                        12.0,
                    );
                    if resp.clicked() {
                        clicked = Some(s.content_id.clone());
                    }
                }
            });
            ui.add_space(gap);
        }
        if let Some(id) = clicked {
            self.activate_station(&id);
        }
    }

    fn empty_hint(&self, ui: &mut egui::Ui, title: &str, body: &str) {
        let (rect, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 190.0), Sense::hover());
        let p = ui.painter().clone();
        let c = Pos2::new(rect.center().x, rect.top() + 50.0);
        p.circle_stroke(c, 26.0, Stroke::new(1.0, theme::white_alpha(28)));
        icons::draw(
            &p,
            Rect::from_center_size(c, Vec2::splat(22.0)),
            Icon::Radio,
            theme::FAINT,
            20.0,
        );
        audit::text(
            &p,
            Pos2::new(c.x, c.y + 44.0),
            Align2::CENTER_CENTER,
            title,
            theme::bold(13.0),
            theme::DIM,
        );
        audit::text(
            &p,
            Pos2::new(c.x, c.y + 66.0),
            Align2::CENTER_CENTER,
            body,
            theme::regular(11.0),
            theme::WEAK,
        );
    }
}
// ------------------------------------------------------------------ 节目单抽屉

impl App {
    fn program_panel(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        let p = ui.painter().clone();
        p.rect_filled(full, CornerRadius::ZERO, theme::PANEL);
        p.line_segment(
            [
                Pos2::new(full.left() + 0.5, full.top()),
                Pos2::new(full.left() + 0.5, full.bottom()),
            ],
            Stroke::new(1.0, theme::white_alpha(16)),
        );

        let station = self
            .now
            .as_ref()
            .map(|n| n.station.clone())
            .or_else(|| self.selected.as_ref().and_then(|id| self.station_by_id(id)));

        // 头部
        let head = Rect::from_min_size(full.min, Vec2::new(full.width(), theme::TOOLBAR_H));
        p.line_segment(
            [
                Pos2::new(head.left(), head.bottom()),
                Pos2::new(head.right(), head.bottom()),
            ],
            Stroke::new(1.0, theme::white_alpha(14)),
        );
        if let Some(s) = &station {
            let cover_rect = Rect::from_center_size(
                Pos2::new(head.left() + 34.0, head.center().y),
                Vec2::splat(36.0),
            );
            let tex = self.images.get(ui.ctx(), &s.image);
            widgets::cover(ui, cover_rect, 8, tex, &s.title);
            let font = theme::bold(12.5);
            let name = widgets::ellipsize(ui.ctx(), &s.title, &font, head.width() - 105.0);
            audit::text(
                &p,
                Pos2::new(cover_rect.right() + 11.0, head.center().y - 8.0),
                Align2::LEFT_CENTER,
                name,
                font,
                theme::TEXT,
            );
            audit::text(
                &p,
                Pos2::new(cover_rect.right() + 11.0, head.center().y + 8.0),
                Align2::LEFT_CENTER,
                format!(
                    "{} · 节目单",
                    if s.tag_name.is_empty() {
                        "电台"
                    } else {
                        &s.tag_name
                    }
                ),
                theme::regular(10.0),
                theme::WEAK,
            );
        } else {
            audit::text(
                &p,
                Pos2::new(head.left() + 16.0, head.center().y),
                Align2::LEFT_CENTER,
                "选择一个电台查看节目单",
                theme::regular(11.5),
                theme::WEAK,
            );
        }

        // 日期 tab
        let tabs = ["今天", "昨天", "前天"];
        let mut x = full.left() + 14.0;
        let y = head.bottom() + 4.0;
        for (i, label) in tabs.iter().enumerate() {
            let font = theme::regular(11.0);
            let w = ui
                .painter()
                .layout_no_wrap(label.to_string(), font.clone(), theme::TEXT)
                .size()
                .x
                + 22.0;
            let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 26.0));
            let resp = {
                audit::ctl(&format!("tab-{i}"), rect, ui.clip_rect());
                ui.interact(rect, ui.scope_id().with(("tab", i)), Sense::click())
            };
            let active = self.program_day == i;
            p.rect(
                rect,
                CornerRadius::same(7),
                if active {
                    theme::ELEV2
                } else if resp.hovered() {
                    theme::white_alpha(8)
                } else {
                    Color32::TRANSPARENT
                },
                Stroke::new(
                    1.0,
                    if active {
                        theme::white_alpha(26)
                    } else {
                        Color32::TRANSPARENT
                    },
                ),
                StrokeKind::Inside,
            );
            audit::text(
                &p,
                rect.center(),
                Align2::CENTER_CENTER,
                *label,
                font,
                if active { theme::TEXT } else { theme::DIM },
            );
            if resp.clicked() && !active {
                self.set_program_day(i);
            }
            x += w + 6.0;
        }

        // 列表
        let list_top = y + 34.0;
        let list_rect = Rect::from_min_max(
            Pos2::new(full.left(), list_top),
            Pos2::new(full.right(), full.bottom() - 30.0),
        );
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(list_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        let mut play_prog: Option<Program> = None;
        let now_id = self.now.as_ref().and_then(|n| n.program_id.clone());

        // 回听中：给一个明确的出口，否则进到回听就回不到直播
        let replaying = self.now.as_ref().map(|n| n.is_replay).unwrap_or(false);
        let mut want_live = false;
        if replaying {
            let font = theme::regular(9.5);
            let label = "回到直播";
            let w_pill = ui
                .painter()
                .layout_no_wrap(label.to_string(), font.clone(), theme::ACCENT)
                .size()
                .x
                + 34.0;
            let pill = Rect::from_center_size(
                Pos2::new(full.right() - 14.0 - w_pill * 0.5, head.bottom() + 17.0),
                Vec2::new(w_pill, 24.0),
            );
            let resp = {
                audit::ctl("panel-back-live", pill, ui.clip_rect());
                ui.interact(pill, ui.scope_id().with("panel-back-live"), Sense::click())
            };
            p.rect(
                pill,
                CornerRadius::same(theme::R_PILL),
                if resp.hovered() {
                    theme::accent_alpha(40)
                } else {
                    theme::accent_alpha(22)
                },
                Stroke::new(1.0, theme::accent_alpha(120)),
                StrokeKind::Inside,
            );
            widgets::live_dot(
                &p,
                Pos2::new(pill.left() + 11.0, pill.center().y),
                2.0,
                self.start.elapsed().as_secs_f32(),
                theme::ACCENT,
            );
            audit::text(
                &p,
                Pos2::new(pill.left() + 19.0, pill.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                theme::ACCENT,
            );
            if resp.clicked() {
                want_live = true;
            }
        }

        let mut area = egui::ScrollArea::vertical()
            .id_salt("program-list")
            .auto_shrink([false, false]);
        if let Some(off) = self.program_scroll.take() {
            area = area.vertical_scroll_offset(off);
        }
        area.show(&mut child, |ui| {
            ui.add_space(6.0);
            if self.program_loading && self.programs.is_empty() {
                for _ in 0..6 {
                    let (rect, _) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
                    ui.painter().rect_filled(
                        rect.shrink2(Vec2::new(12.0, 6.0)),
                        CornerRadius::same(6),
                        theme::white_alpha(6),
                    );
                }
            }
            for prog in &self.programs {
                let is_now = prog.is_live_now() && self.program_day == 0;
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
                let p = ui.painter().clone();
                let playing = now_id.as_deref() == Some(prog.id.as_str());
                if playing {
                    p.rect_filled(
                        rect.shrink2(Vec2::new(8.0, 2.0)),
                        CornerRadius::same(8),
                        theme::accent_alpha(30),
                    );
                    p.rect_filled(
                        Rect::from_min_size(
                            Pos2::new(rect.left() + 8.0, rect.top() + 8.0),
                            Vec2::new(2.0, rect.height() - 16.0),
                        ),
                        CornerRadius::same(1),
                        theme::ACCENT,
                    );
                } else if resp.hovered() {
                    p.rect_filled(
                        rect.shrink2(Vec2::new(8.0, 2.0)),
                        CornerRadius::same(8),
                        theme::white_alpha(9),
                    );
                }
                audit::text(
                    &p,
                    Pos2::new(rect.left() + 16.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    widgets::fmt_clock(&prog.start_local()),
                    theme::regular(10.0),
                    if is_now || playing {
                        theme::ACCENT
                    } else {
                        theme::WEAK
                    },
                );
                let name_x = rect.left() + 56.0;
                let name_font = if is_now || playing {
                    theme::bold(11.5)
                } else {
                    theme::regular(11.5)
                };
                let avail = rect.width() - 56.0 - 54.0;
                let name = widgets::ellipsize(ui.ctx(), &prog.program_name, &name_font, avail);
                audit::text(
                    &p,
                    Pos2::new(name_x, rect.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    name_font,
                    if is_now || playing {
                        theme::TEXT
                    } else {
                        Color32::from_rgb(0xc0, 0xc5, 0xcb)
                    },
                );
                if is_now && replaying {
                    let label = "回到直播";
                    let font = theme::regular(9.0);
                    let w = ui
                        .painter()
                        .layout_no_wrap(label.to_string(), font.clone(), theme::ACCENT)
                        .size()
                        .x
                        + 26.0;
                    let pill = Rect::from_center_size(
                        Pos2::new(rect.right() - 14.0 - w * 0.5, rect.center().y),
                        Vec2::new(w, 18.0),
                    );
                    p.rect(
                        pill,
                        CornerRadius::same(theme::R_PILL),
                        theme::accent_alpha(26),
                        Stroke::new(1.0, theme::accent_alpha(120)),
                        StrokeKind::Inside,
                    );
                    audit::text(
                        &p,
                        pill.center(),
                        Align2::CENTER_CENTER,
                        label,
                        font,
                        theme::ACCENT,
                    );
                    if resp.clicked() {
                        want_live = true;
                    }
                } else if is_now {
                    // 圆点要和下面各行「时长」那一列对齐：时长右对齐到 right-18，
                    // 所以点心落在 right-26（正文列中心），而不是原来的 right-42 飘在左边
                    let dot = Pos2::new(rect.right() - 24.0, rect.center().y);
                    widgets::live_dot(
                        &p,
                        dot,
                        2.2,
                        self.start.elapsed().as_secs_f32(),
                        theme::ACCENT,
                    );
                } else if resp.hovered() {
                    let label = "回听";
                    let font = theme::regular(9.5);
                    let w = 40.0;
                    let pill = Rect::from_center_size(
                        Pos2::new(rect.right() - 34.0, rect.center().y),
                        Vec2::new(w, 18.0),
                    );
                    ui.painter().rect(
                        pill,
                        CornerRadius::same(theme::R_PILL),
                        theme::amber_alpha(24),
                        Stroke::new(1.0, theme::amber_alpha(70)),
                        StrokeKind::Inside,
                    );
                    audit::text(
                        ui.painter(),
                        pill.center(),
                        Align2::CENTER_CENTER,
                        label,
                        font,
                        theme::AMBER,
                    );
                } else {
                    audit::text(
                        &p,
                        Pos2::new(rect.right() - 18.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        format!("{}′", (prog.duration_secs() / 60).max(1)),
                        theme::regular(9.5),
                        theme::FAINT,
                    );
                }
                // 回听进度条
                if let Some(pos) = self.store.progress.get(&prog.id) {
                    let frac = (*pos / prog.duration_secs().max(1) as f64).clamp(0.0, 1.0) as f32;
                    let bar = Rect::from_min_size(
                        Pos2::new(name_x, rect.bottom() - 5.0),
                        Vec2::new(avail, 2.0),
                    );
                    ui.painter().rect_filled(bar, 1.0, theme::white_alpha(16));
                    ui.painter().rect_filled(
                        Rect::from_min_size(bar.min, Vec2::new(bar.width() * frac, 2.0)),
                        1.0,
                        theme::amber_alpha(160),
                    );
                }
                if resp.clicked() && !(is_now && replaying) {
                    play_prog = Some(prog.clone());
                }
            }
            ui.add_space(12.0);
        });

        if want_live {
            self.back_to_live();
        }
        if let Some(prog) = play_prog {
            self.play_program(prog);
        }

        // 底部
        audit::text(
            &p,
            Pos2::new(full.left() + 16.0, full.bottom() - 16.0),
            Align2::LEFT_CENTER,
            "可回溯 30 天 · 点节目回听",
            theme::regular(9.5),
            theme::FAINT,
        );
    }

    /// 今天：把列表滚到当前节目附近；其它日期回到顶部
    fn scroll_to_now(&self) -> Option<f32> {
        if self.program_day != 0 {
            return Some(0.0);
        }
        let now = chrono::Local::now().timestamp_millis();
        let idx = self
            .programs
            .iter()
            .position(|p| p.start_time <= now && now < p.end_time)?;
        Some((idx as f32 * 36.0 - 108.0).max(0.0))
    }

    fn set_program_day(&mut self, day: usize) {
        self.program_day = day;
        let today = chrono::Local::now().date_naive();
        self.program_date = today - chrono::Duration::days(day as i64);
        let Some(id) = self.selected.clone() else {
            return;
        };
        let date = self.program_date.format("%Y-%m-%d").to_string();
        if let Some(cached) = self.client.programs_cached(&id, &date) {
            self.programs = cached;
            self.program_loading = false;
            return;
        }
        self.program_loading = true;
        self.programs.clear();
        self.program_scroll = Some(0.0);
        self.spawn(move |c| match c.programs(&id, &date) {
            Ok((list, _)) => Msg::Programs(list),
            Err(_) => Msg::Programs(Vec::new()),
        });
    }
}

// ------------------------------------------------------------------ 播放条

impl App {
    fn player_bar(&mut self, ui: &mut egui::Ui) {
        let full = ui.max_rect();
        let p = ui.painter().clone();
        p.line_segment(
            [
                Pos2::new(full.left(), full.top() + 0.5),
                Pos2::new(full.right(), full.top() + 0.5),
            ],
            Stroke::new(1.0, theme::white_alpha(18)),
        );
        widgets::gradient_v(
            &p,
            full,
            Color32::from_rgb(0x12, 0x14, 0x1a),
            Color32::from_rgb(0x0c, 0x0e, 0x12),
        );

        let snap = self.engine.snapshot();
        let now = self.now.clone();

        // 左：当前台（未播放时显示已选中的台）
        let left_w = 292.0;
        let mut x = full.left() + 14.0;
        let idle_station = now
            .as_ref()
            .map(|n| (n.station.clone(), n.program_name.clone()))
            .or_else(|| {
                self.selected.as_ref().and_then(|id| {
                    self.station_by_id(id).map(|s| {
                        let name = self
                            .detail
                            .as_ref()
                            .and_then(|d| d.living_program_name.clone())
                            .unwrap_or_else(|| s.now_playing().to_string());
                        (s, name)
                    })
                })
            });
        if let Some(now) = &now {
            let cover_rect =
                Rect::from_center_size(Pos2::new(x + 25.0, full.center().y), Vec2::splat(50.0));
            let tex = self.images.get(ui.ctx(), &now.station.image);
            widgets::cover(ui, cover_rect, 9, tex, &now.station.title);
            let title_font = theme::bold(13.0);
            let title = widgets::ellipsize(ui.ctx(), &now.station.title, &title_font, 150.0);
            audit::text(
                &p,
                Pos2::new(cover_rect.right() + 11.0, full.center().y - 9.0),
                Align2::LEFT_CENTER,
                title,
                title_font,
                theme::TEXT,
            );
            let sub_font = theme::regular(10.5);
            let sub = widgets::ellipsize(ui.ctx(), &now.program_name, &sub_font, 150.0);
            audit::text(
                &p,
                Pos2::new(cover_rect.right() + 11.0, full.center().y + 9.0),
                Align2::LEFT_CENTER,
                if snap.state == PlayState::Paused {
                    "已暂停".to_string()
                } else {
                    sub
                },
                sub_font,
                if snap.state == PlayState::Paused {
                    theme::AMBER
                } else {
                    theme::WEAK
                },
            );
            let heart = Rect::from_center_size(
                Pos2::new(full.left() + left_w - 18.0, full.center().y),
                Vec2::splat(26.0),
            );
            let hresp = {
                let _r = heart;
                audit::ctl("pb-heart", _r, ui.clip_rect());
                ui.interact(_r, ui.scope_id().with("pb-heart"), Sense::click())
            };
            let faved = self.store.is_fav(&now.station.content_id);
            icons::draw(
                &p,
                heart,
                if faved { Icon::HeartFill } else { Icon::Heart },
                if faved {
                    theme::ACCENT
                } else if hresp.hovered() {
                    theme::TEXT
                } else {
                    theme::WEAK
                },
                18.0,
            );
            if hresp.clicked() {
                let s = now.station.clone();
                self.store.toggle_fav(&s);
                self.save_store();
            }
        } else if let Some((station, program)) = idle_station {
            let cover_rect =
                Rect::from_center_size(Pos2::new(x + 24.0, full.center().y), Vec2::splat(48.0));
            let tex = self.images.get(ui.ctx(), &station.image);
            widgets::cover(ui, cover_rect, 9, tex, &station.title);
            let title_font = theme::bold(13.0);
            let title = widgets::ellipsize(ui.ctx(), &station.title, &title_font, 150.0);
            audit::text(
                &p,
                Pos2::new(cover_rect.right() + 11.0, full.center().y - 9.0),
                Align2::LEFT_CENTER,
                title,
                title_font,
                theme::TEXT,
            );
            let sub_font = theme::regular(10.5);
            let sub = widgets::ellipsize(ui.ctx(), &program, &sub_font, 150.0);
            audit::text(
                &p,
                Pos2::new(cover_rect.right() + 11.0, full.center().y + 9.0),
                Align2::LEFT_CENTER,
                sub,
                sub_font,
                theme::WEAK,
            );
        } else {
            audit::text(
                &p,
                Pos2::new(x + 8.0, full.center().y),
                Align2::LEFT_CENTER,
                "未在播放",
                theme::regular(11.5),
                theme::WEAK,
            );
        }
        x += left_w;

        // 中：传输控制
        let cy = full.center().y;
        let prev_rect = Rect::from_center_size(Pos2::new(x + 16.0, cy), Vec2::splat(30.0));
        let play_rect = Rect::from_center_size(Pos2::new(x + 58.0, cy), Vec2::splat(40.0));
        let next_rect = Rect::from_center_size(Pos2::new(x + 100.0, cy), Vec2::splat(30.0));
        let prev_resp = {
            let _r = prev_rect;
            audit::ctl("pb-prev", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-prev"), Sense::click())
        };
        let play_resp = {
            let _r = play_rect;
            audit::ctl("pb-play", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-play"), Sense::click())
        };
        let next_resp = {
            let _r = next_rect;
            audit::ctl("pb-next", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-next"), Sense::click())
        };

        icons::draw(
            &p,
            prev_rect,
            Icon::Prev,
            if prev_resp.hovered() {
                theme::TEXT
            } else {
                theme::DIM
            },
            20.0,
        );
        p.circle_filled(
            play_rect.center(),
            20.0,
            if play_resp.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(0xf2, 0xf3, 0xf5)
            },
        );
        icons::draw(
            &p,
            play_rect,
            if snap.state == PlayState::Playing {
                Icon::Pause
            } else {
                Icon::Play
            },
            Color32::from_rgb(0x10, 0x12, 0x16),
            20.0,
        );
        icons::draw(
            &p,
            next_rect,
            Icon::Next,
            if next_resp.hovered() {
                theme::TEXT
            } else {
                theme::DIM
            },
            20.0,
        );
        if play_resp.clicked() {
            self.toggle_play();
        }
        if prev_resp.clicked() {
            self.step_station(-1);
        }
        if next_resp.clicked() {
            self.step_station(1);
        }

        // 右侧控件组的真实宽度先算出来：进度条绝不能越到它下面
        let pill_font = theme::regular(10.0);
        let q_text = format!(
            "音质 {}",
            if self.store.quality_high {
                "高清"
            } else {
                "标清"
            }
        );
        let l_text = "线路 自动".to_string();
        let q_w = p
            .layout_no_wrap(q_text.clone(), pill_font.clone(), theme::DIM)
            .size()
            .x
            + 20.0;
        let l_w = p
            .layout_no_wrap(l_text.clone(), pill_font.clone(), theme::DIM)
            .size()
            .x
            + 20.0;
        let sleep_w = 62.0;
        let vol_w = 84.0;
        let vol_icon_w = 26.0;
        let g = 8.0;
        let right_w = 14.0
            + 28.0
            + 6.0
            + 28.0
            + g
            + vol_icon_w
            + vol_w
            + g
            + sleep_w
            + g
            + l_w
            + g
            + q_w
            + 14.0;
        let right_left = full.right() - right_w;

        // 中右：状态 / 进度（宽度按右侧控件组的左边界收口）
        let status_left = x + 124.0;
        let status_w = (right_left - status_left - 14.0).max(120.0);
        let is_replay = now.as_ref().map(|n| n.is_replay).unwrap_or(false);

        if now.is_none() {
            audit::text(
                &p,
                Pos2::new(status_left, cy),
                Align2::LEFT_CENTER,
                "点任意电台开始收听",
                theme::regular(10.5),
                theme::FAINT,
            );
        } else if is_replay {
            let dur = snap
                .duration
                .or(now.as_ref().and_then(|n| n.duration))
                .unwrap_or(0.0);
            let pos = self.seek_preview.unwrap_or(snap.position);
            let frac = if dur > 0.0 {
                (pos / dur).clamp(0.0, 1.0) as f32
            } else {
                0.0
            };
            audit::text(
                &p,
                Pos2::new(status_left, cy - 13.0),
                Align2::LEFT_CENTER,
                widgets::fmt_time(pos),
                theme::regular(10.0),
                theme::DIM,
            );
            audit::text(
                &p,
                Pos2::new(status_left + status_w, cy - 13.0),
                Align2::RIGHT_CENTER,
                widgets::fmt_time(dur),
                theme::regular(10.0),
                theme::WEAK,
            );
            let bar =
                Rect::from_min_size(Pos2::new(status_left, cy + 4.0), Vec2::new(status_w, 4.0));
            let resp = {
                let _r = bar.expand2(Vec2::new(0.0, 8.0));
                audit::ctl("回听进度条", _r, ui.clip_rect());
                ui.interact(_r, ui.scope_id().with("pb-seek"), Sense::click_and_drag())
            };
            if resp.dragged() || resp.clicked() {
                if let Some(pointer) = ui.ctx().pointer_interact_pos() {
                    let f = ((pointer.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                    self.seek_preview = Some(f as f64 * dur);
                }
            }
            if resp.drag_stopped() {
                if let Some(target) = self.seek_preview.take() {
                    self.engine.seek(target);
                }
            }
            widgets::progress(&p, bar, frac, theme::white_alpha(22), theme::AMBER, true);
        } else {
            let live = format!(
                "LIVE · 延迟约 12 秒 · {}",
                now.as_ref()
                    .map(|n| n.line_label.clone())
                    .unwrap_or_default()
            );
            widgets::live_dot(
                &p,
                Pos2::new(status_left + 3.0, cy - 13.0),
                2.4,
                self.start.elapsed().as_secs_f32(),
                theme::ACCENT,
            );
            audit::text(
                &p,
                Pos2::new(status_left + 12.0, cy - 13.0),
                Align2::LEFT_CENTER,
                live,
                theme::regular(10.0),
                theme::DIM,
            );
            let bar =
                Rect::from_min_size(Pos2::new(status_left, cy + 4.0), Vec2::new(status_w, 3.0));
            audit::ctl(
                "直播进度条",
                bar.expand2(Vec2::new(0.0, 3.0)),
                ui.clip_rect(),
            );
            p.rect_filled(bar, 1.5, theme::white_alpha(20));
            let t = self.start.elapsed().as_secs_f32();
            let w = (bar.width() * (0.25 + 0.5 * (0.5 + 0.5 * (t * 0.6).sin())))
                .clamp(40.0, bar.width());
            widgets::gradient_h(
                &p,
                Rect::from_min_size(bar.min, Vec2::new(w, bar.height())),
                theme::accent_alpha(60),
                theme::ACCENT,
            );
        }

        // 右：音质 | 线路 | 定时 | 音量 | 迷你 | 面板
        // 统一从 right_left 向右顺序排，每一步都推进 cx，绝不回退扣减
        let mut cx = right_left + 14.0;

        let q_rect = Rect::from_min_size(Pos2::new(cx, cy - 12.0), Vec2::new(q_w, 24.0));
        let q_resp = {
            let _r = q_rect;
            audit::ctl("pb-quality", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-quality"), Sense::click())
        };
        p.rect(
            q_rect,
            CornerRadius::same(7),
            if q_resp.hovered() {
                theme::white_alpha(10)
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(1.0, theme::white_alpha(20)),
            StrokeKind::Inside,
        );
        audit::text(
            &p,
            q_rect.center(),
            Align2::CENTER_CENTER,
            q_text,
            pill_font.clone(),
            theme::DIM,
        );
        let q_resp = q_resp.on_hover_text("点击切换音质");
        egui::Popup::menu(&q_resp).show(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(10.0, 5.0);
            ui.set_min_width(150.0);
            if ui
                .selectable_label(self.store.quality_high, "高清（AAC 132k+）")
                .clicked()
            {
                self.store.quality_high = true;
                self.save_store();
                ui.close();
            }
            if ui
                .selectable_label(!self.store.quality_high, "标清（省流量）")
                .clicked()
            {
                self.store.quality_high = false;
                self.save_store();
                ui.close();
            }
        });
        cx += q_w + g;

        let l_rect = Rect::from_min_size(Pos2::new(cx, cy - 12.0), Vec2::new(l_w, 24.0));
        let l_resp = {
            let _r = l_rect;
            audit::ctl("pb-line", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-line"), Sense::click())
        };
        p.rect(
            l_rect,
            CornerRadius::same(7),
            if l_resp.hovered() {
                theme::white_alpha(10)
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(1.0, theme::white_alpha(20)),
            StrokeKind::Inside,
        );
        audit::text(
            &p,
            l_rect.center(),
            Align2::CENTER_CENTER,
            l_text,
            pill_font.clone(),
            theme::DIM,
        );
        let l_resp = l_resp.on_hover_text("点击查看线路选择规则");
        egui::Popup::menu(&l_resp).show(|ui| {
            ui.set_min_width(170.0);
            ui.label(
                RichText::new("线路由播放器自动选择：")
                    .size(10.0)
                    .color(theme::WEAK),
            );
            ui.label(
                RichText::new("高清 → 备用 → 标清")
                    .size(10.0)
                    .color(theme::DIM),
            );
        });
        cx += l_w + g;

        // 睡眠定时
        let sleep_label = if let Some(deadline) = self.sleep_until {
            let left = deadline.saturating_duration_since(Instant::now()).as_secs() / 60;
            format!("{} 分", left.max(1))
        } else {
            "定时".to_string()
        };
        let sleep_rect = Rect::from_min_size(Pos2::new(cx, cy - 12.0), Vec2::new(sleep_w, 24.0));
        let sresp = {
            let _r = sleep_rect;
            audit::ctl("pb-sleep", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-sleep"), Sense::click())
        };
        p.rect(
            sleep_rect,
            CornerRadius::same(7),
            if self.sleep_until.is_some() {
                theme::amber_alpha(22)
            } else {
                Color32::TRANSPARENT
            },
            Stroke::new(
                1.0,
                if self.sleep_until.is_some() {
                    theme::amber_alpha(70)
                } else {
                    theme::white_alpha(20)
                },
            ),
            StrokeKind::Inside,
        );
        audit::text(
            &p,
            sleep_rect.center(),
            Align2::CENTER_CENTER,
            sleep_label,
            theme::regular(10.0),
            if self.sleep_until.is_some() {
                theme::AMBER
            } else {
                theme::WEAK
            },
        );
        let sresp = sresp.on_hover_text("点击设置睡眠定时");
        let sleep_toggle = sresp.clicked().then_some(egui::SetOpenCommand::Toggle);
        egui::Popup::menu(&sresp)
            .open_memory(sleep_toggle)
            .show(|ui| {
                ui.set_min_width(120.0);
                for m in [15, 30, 45, 60, 90] {
                    if ui.button(format!("{m} 分钟后停止")).clicked() {
                        self.sleep_minutes = m;
                        self.sleep_until =
                            Some(Instant::now() + Duration::from_secs(m as u64 * 60));
                        ui.close();
                    }
                }
                if self.sleep_until.is_some() && ui.button("取消定时").clicked() {
                    self.sleep_until = None;
                    ui.close();
                }
            });
        cx += sleep_w + g;

        // 音量：图标 + 滑杆，占位 26 + 84
        let vicon = Rect::from_center_size(Pos2::new(cx + vol_icon_w * 0.5, cy), Vec2::splat(17.0));
        let vol_rect = Rect::from_min_size(
            Pos2::new(cx + vol_icon_w, cy - 10.0),
            Vec2::new(vol_w, 20.0),
        );
        let vresp = {
            let _r = Rect::from_min_max(vicon.min, vol_rect.max).expand2(Vec2::new(2.0, 6.0));
            audit::ctl("pb-vol", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-vol"), Sense::click_and_drag())
        };
        icons::draw(
            &p,
            vicon,
            if self.muted || self.volume < 0.01 {
                Icon::Mute
            } else if self.volume < 0.5 {
                Icon::VolumeLow
            } else {
                Icon::Volume
            },
            if vresp.hovered() {
                theme::TEXT
            } else {
                theme::DIM
            },
            17.0,
        );
        let track = Rect::from_center_size(vol_rect.center(), Vec2::new(vol_w, 3.0));
        let over_icon = ui
            .ctx()
            .pointer_interact_pos()
            .map(|q| vicon.expand(4.0).contains(q))
            .unwrap_or(false);
        if vresp.dragged() || (vresp.clicked() && !over_icon) {
            if let Some(pointer) = ui.ctx().pointer_interact_pos() {
                let f = ((pointer.x - track.left()) / track.width()).clamp(0.0, 1.0);
                self.volume = f;
                self.muted = false;
                self.engine.set_volume(f);
            }
        }
        if vresp.clicked() && over_icon {
            self.muted = !self.muted;
            self.engine
                .set_volume(if self.muted { 0.0 } else { self.volume });
        }
        widgets::progress(
            &p,
            track,
            self.volume,
            theme::white_alpha(26),
            Color32::from_rgb(0x8b, 0x90, 0x99),
            true,
        );
        cx += vol_icon_w + vol_w + g;

        // 迷你
        let mini_btn = Rect::from_center_size(Pos2::new(cx + 14.0, cy), Vec2::splat(28.0));
        let mresp = {
            let _r = mini_btn;
            audit::ctl("pb-mini", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-mini"), Sense::click())
        };
        icons::draw(
            &p,
            mini_btn,
            Icon::Mini,
            if mresp.hovered() {
                theme::TEXT
            } else {
                theme::WEAK
            },
            18.0,
        );
        if mresp.clicked() {
            self.mini_mode(ui.ctx());
        }
        cx += 28.0 + 6.0;

        // 节目单开关
        let panel_btn = Rect::from_center_size(Pos2::new(cx + 14.0, cy), Vec2::splat(28.0));
        let presp = {
            let _r = panel_btn;
            audit::ctl("pb-panel", _r, ui.clip_rect());
            ui.interact(_r, ui.scope_id().with("pb-panel"), Sense::click())
        };
        icons::draw(
            &p,
            panel_btn,
            Icon::Panel,
            if self.panel_open {
                theme::TEXT
            } else if presp.hovered() {
                theme::DIM
            } else {
                theme::WEAK
            },
            18.0,
        );
        if presp.clicked() {
            self.panel_open = !self.panel_open;
            self.save_store();
        }
    }

    /// `--replay`：等直播真的起来、节目单到手之后再切回听（否则被直播的 Resolved 覆盖）
    fn pending_replay_tick(&mut self) {
        if !self.pending_replay || self.now.is_none() || self.programs.is_empty() {
            return;
        }
        self.pending_replay = false;
        let now = chrono::Local::now().timestamp_millis();
        if let Some(p) = self
            .programs
            .iter()
            .rev()
            .find(|p| p.end_time <= now)
            .cloned()
        {
            self.play_program(p);
        }
    }

    /// `--check-live`：走一遍「回听 → 回到直播」，断言状态真的切回来了
    fn live_check_tick(&mut self, ctx: &egui::Context) {
        if self.live_check == 0 {
            return;
        }
        ctx.request_repaint();
        // 等节目单/详情就绪
        if self.live_check == 1 {
            if self.programs.is_empty() || self.now.is_none() {
                return;
            }
            let now = chrono::Local::now().timestamp_millis();
            // 取最近一档「已开始且有回听地址」的节目；刚过零点时当天可能一档都没有，
            // 就切到「昨天」再找
            let past = self
                .programs
                .iter()
                .rev()
                .find(|p| p.start_time <= now && p.replay_url().is_some())
                .cloned();
            match past {
                Some(p) => {
                    self.play_program(p);
                    self.live_check = 2;
                    self.live_check_t = Instant::now();
                }
                None => {
                    if self.program_day == 0 {
                        self.set_program_day(1);
                        self.live_check_t = Instant::now();
                    } else {
                        println!("FAIL 找不到可回听的节目，无法验证");
                        std::process::exit(1);
                    }
                }
            }
            return;
        }
        if self.live_check_t.elapsed() < Duration::from_millis(1800) {
            return;
        }
        if self.live_check == 2 {
            let replaying = self.now.as_ref().map(|n| n.is_replay).unwrap_or(false);
            if !replaying {
                println!("FAIL 点节目后没有进入回听模式");
                std::process::exit(1);
            }
            let snap = self.engine.snapshot();
            println!(
                "PASS 点节目进入回听                 state={:?} pos={:.1}s",
                snap.state, snap.position
            );
            self.back_to_live();
            self.live_check = 3;
            self.live_check_t = Instant::now();
            return;
        }
        if self.live_check == 3 {
            let replaying = self.now.as_ref().map(|n| n.is_replay).unwrap_or(false);
            let snap = self.engine.snapshot();
            if replaying || snap.state != PlayState::Playing {
                println!(
                    "FAIL 回到直播失败               replay={replaying} state={:?}",
                    snap.state
                );
                std::process::exit(1);
            }
            println!(
                "PASS 回到直播                       state={:?} 直播线={}",
                snap.state,
                self.now
                    .as_ref()
                    .map(|n| n.line_label.clone())
                    .unwrap_or_default()
            );
            let id = self
                .now
                .as_ref()
                .map(|n| n.station.content_id.clone())
                .unwrap_or_default();
            self.activate_station(&id);
            self.live_check = 4;
            self.live_check_t = Instant::now();
            return;
        }
        if self.live_check == 4 {
            let snap = self.engine.snapshot();
            if snap.state == PlayState::Playing {
                println!(
                    "FAIL 点正在播的卡片没有暂停           state={:?}",
                    snap.state
                );
                std::process::exit(1);
            }
            println!("PASS 点正在播的卡片 → 暂停          state={:?}", snap.state);
            let id = self
                .now
                .as_ref()
                .map(|n| n.station.content_id.clone())
                .unwrap_or_default();
            self.activate_station(&id);
            self.live_check = 5;
            self.live_check_t = Instant::now();
            return;
        }
        if self.live_check == 5 {
            let snap = self.engine.snapshot();
            if snap.state != PlayState::Playing {
                println!(
                    "FAIL 再点一次没有继续播放             state={:?}",
                    snap.state
                );
                std::process::exit(1);
            }
            println!("PASS 再点一次 → 继续播放            state={:?}", snap.state);
            println!("\n全部通过（4 项）");
            std::process::exit(0);
        }
    }

    /// `--audit` 模式：自动轮换每个页面，收集重叠并打印后退出
    fn audit_tick(&mut self, ctx: &egui::Context) {
        if !audit::is_enabled() {
            return;
        }
        // 自检模式必须持续出帧，否则时间不推进、永远走不到下一屏
        ctx.request_repaint();
        if self.all.is_empty() {
            return;
        }
        let t = ctx.input(|i| i.time);
        if self.audit_t0 == 0.0 {
            self.audit_t0 = t;
        }
        let step = ((t - self.audit_t0) / 0.45) as usize;
        if step >= 1 && !self.audit_probed {
            self.audit_probed = true;
            audit::selftest();
            // 自检必须让播放器真的在播，否则「直播进度条」这类只在播放时出现的元素查不到
            if self.now.is_none() {
                if let Some(s) = self.national.first().cloned() {
                    self.select_station(&s.content_id, true);
                }
            }
        }
        if step == self.audit_step {
            return;
        }
        self.audit_step = step;
        let views = [
            View::Discover,
            View::National,
            View::All,
            View::Region,
            View::Favorites,
            View::Recent,
            View::Search,
            View::Settings,
        ];
        let total = views.len() + 2;
        let idx = step % total;
        if idx < views.len() {
            self.view = views[idx];
            if self.view == View::Region {
                if let Some(p) = self.provinces.first().cloned() {
                    if self.prov != p.code {
                        self.open_province(p.code);
                    }
                }
            }
            if self.view == View::Search && self.results.is_empty() {
                self.search_input = "交通".to_string();
                self.run_search();
            }
        } else {
            self.panel_open = idx == views.len();
        }
        if step > total * 2 || t - self.audit_t0 > 40.0 {
            audit::report();
            std::process::exit(0);
        }
        ctx.request_repaint();
    }

    /// 从回听切回直播：重新解析该台并播放实时流
    fn back_to_live(&mut self) {
        if let Some(n) = self.now.clone() {
            let id = n.station.content_id.clone();
            self.select_station(&id, true);
            self.toast("已回到直播");
        }
    }

    /// 卡片/行的点击语义：点「正在播的这台」是暂停/继续，点别的台才是换台。
    /// 之前一律 select_station(…, true)，所以点播放中的卡片只会重新起播。
    fn activate_station(&mut self, id: &str) {
        let same = self
            .now
            .as_ref()
            .map(|n| n.station.content_id == id)
            .unwrap_or(false);
        if same {
            self.toggle_play();
        } else {
            self.select_station(id, true);
        }
    }

    fn step_station(&mut self, dir: i32) {
        let list: Vec<Station> = if self.view == View::National || !self.national.is_empty() {
            self.national.clone()
        } else {
            self.all.clone()
        };
        if list.is_empty() {
            return;
        }
        let cur = self.now.as_ref().map(|n| n.station.content_id.clone());
        let idx = cur
            .as_ref()
            .and_then(|id| list.iter().position(|s| &s.content_id == id))
            .unwrap_or(0) as i32;
        let next = (idx + dir).rem_euclid(list.len() as i32) as usize;
        let id = list[next].content_id.clone();
        self.select_station(&id, true);
    }

    fn mini_mode(&mut self, ctx: &egui::Context) {
        let monitor = ctx.input(|i| i.viewport().monitor_size);
        let _ = monitor;
        if self.panel_before_mini.is_none() {
            self.panel_before_mini = Some(self.panel_open);
        }
        self.panel_open = false;
        self.mini_pending = true;
        ctx.request_repaint();
        self.toast("迷你窗口：拖动窗口边缘可放大回来");
    }
}

// ------------------------------------------------------------------ 浮层

impl App {
    fn toast_overlay(&mut self, ctx: &egui::Context) {
        let Some((msg, at)) = self.toast.clone() else {
            return;
        };
        if at.elapsed() > Duration::from_secs(3) {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(200));
        egui::Area::new(egui::Id::unique("toast"))
            .order(egui::Order::Tooltip)
            .anchor(
                egui::Align2::CENTER_BOTTOM,
                Vec2::new(0.0, -(theme::PLAYER_H + 16.0)),
            )
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgb(0x1a, 0x1d, 0x24))
                    .corner_radius(CornerRadius::same(theme::R_PILL))
                    .stroke(Stroke::new(1.0, theme::white_alpha(30)))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 6],
                        blur: 20,
                        spread: 0,
                        color: Color32::from_black_alpha(120),
                    })
                    .inner_margin(egui::Margin::symmetric(16, 9))
                    .show(ui, |ui| {
                        ui.label(RichText::new(msg).size(11.5).color(theme::TEXT));
                    });
            });
    }

    fn splash(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::unique("splash"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgb(0x12, 0x14, 0x19))
                    .corner_radius(CornerRadius::same(14))
                    .stroke(Stroke::new(1.0, theme::white_alpha(24)))
                    .inner_margin(egui::Margin::symmetric(28, 22))
                    .show(ui, |ui| {
                        let t = self.start.elapsed().as_secs_f32();
                        ui.horizontal(|ui| {
                            widgets::live_dot(
                                ui.painter(),
                                ui.cursor().min + Vec2::new(4.0, 9.0),
                                3.0,
                                t,
                                theme::ACCENT,
                            );
                            ui.add_space(16.0);
                            ui.label(
                                RichText::new("正在整理全国电台…")
                                    .size(12.5)
                                    .color(theme::TEXT),
                            );
                        });
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new(
                                "首次启动会合并央广 19 个频率与 31 个省的电台，之后走本地缓存",
                            )
                            .size(10.0)
                            .color(theme::WEAK),
                        );
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(120));
    }
}
// ------------------------------------------------------------------ 迷你窗口

impl App {
    /// 自绘标题栏：品牌 + 拖拽区；macOS 左侧留出系统红绿灯，其它平台右侧画窗口按钮
    fn titlebar(&mut self, ui: &mut egui::Ui) {
        let rect =
            Rect::from_min_size(ui.cursor().min, Vec2::new(ui.available_width(), TITLEBAR_H));
        let p = ui.painter().clone();
        p.rect_filled(rect, CornerRadius::ZERO, theme::PANEL);
        p.line_segment(
            [
                Pos2::new(rect.left(), rect.bottom()),
                Pos2::new(rect.right(), rect.bottom()),
            ],
            Stroke::new(1.0, theme::white_alpha(14)),
        );

        let mac = cfg!(target_os = "macos");
        // macOS 左侧留给系统红绿灯，其它平台从 0 开始
        let left = if mac { MAC_LIGHTS_W } else { 0.0 };

        let buttons_w = if mac { 0.0 } else { 46.0 * 3.0 };
        window_drag(
            ui,
            Rect::from_min_max(
                Pos2::new(rect.left() + left, rect.top()),
                Pos2::new(rect.right() - buttons_w, rect.bottom()),
            ),
            "titlebar-drag",
        );

        if !mac {
            let bw = 46.0;
            let maxed = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
            for (i, name) in ["win-min", "win-max", "win-close"].into_iter().enumerate() {
                let r = Rect::from_min_size(
                    Pos2::new(rect.right() - buttons_w + i as f32 * bw, rect.top()),
                    Vec2::new(bw, TITLEBAR_H),
                );
                audit::ctl(name, r, ui.clip_rect());
                let resp = ui.interact(r, ui.scope_id().with(name), Sense::click());
                let close = name == "win-close";
                if resp.hovered() {
                    p.rect_filled(
                        r,
                        CornerRadius::ZERO,
                        if close {
                            Color32::from_rgb(0xc4, 0x2b, 0x1c)
                        } else {
                            theme::white_alpha(16)
                        },
                    );
                }
                let c = if resp.hovered() {
                    theme::TEXT
                } else {
                    theme::DIM
                };
                let ctr = r.center();
                match name {
                    "win-min" => {
                        p.line_segment(
                            [Pos2::new(ctr.x - 5.0, ctr.y), Pos2::new(ctr.x + 5.0, ctr.y)],
                            Stroke::new(1.0, c),
                        );
                    }
                    "win-max" if maxed => {
                        p.rect_stroke(
                            Rect::from_center_size(ctr + Vec2::new(-1.5, 1.5), Vec2::splat(8.0)),
                            CornerRadius::ZERO,
                            Stroke::new(1.0, c),
                            StrokeKind::Inside,
                        );
                        p.rect_stroke(
                            Rect::from_center_size(ctr + Vec2::new(1.5, -1.5), Vec2::splat(8.0)),
                            CornerRadius::ZERO,
                            Stroke::new(1.0, c),
                            StrokeKind::Inside,
                        );
                    }
                    "win-max" => {
                        p.rect_stroke(
                            Rect::from_center_size(ctr, Vec2::splat(9.0)),
                            CornerRadius::ZERO,
                            Stroke::new(1.0, c),
                            StrokeKind::Inside,
                        );
                    }
                    _ => {
                        p.line_segment(
                            [ctr + Vec2::new(-4.5, -4.5), ctr + Vec2::new(4.5, 4.5)],
                            Stroke::new(1.0, c),
                        );
                        p.line_segment(
                            [ctr + Vec2::new(4.5, -4.5), ctr + Vec2::new(-4.5, 4.5)],
                            Stroke::new(1.0, c),
                        );
                    }
                }
                if resp.clicked() {
                    match name {
                        "win-min" => ui
                            .ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
                        "win-max" => ui
                            .ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed)),
                        _ => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                    }
                }
            }
        }

        ui.allocate_space(Vec2::new(rect.width(), TITLEBAR_H));
    }

    /// 无边框窗口的四边/四角缩放热区
    fn resize_edges(&self, ui: &mut egui::Ui) {
        if ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let r = ui.max_rect();
        let e = 5.0;
        let zones: [(Rect, egui::ResizeDirection, &str); 8] = [
            (
                Rect::from_min_max(r.left_top(), Pos2::new(r.right(), r.top() + e)),
                egui::ResizeDirection::North,
                "rz-n",
            ),
            (
                Rect::from_min_max(Pos2::new(r.left(), r.bottom() - e), r.right_bottom()),
                egui::ResizeDirection::South,
                "rz-s",
            ),
            (
                Rect::from_min_max(r.left_top(), Pos2::new(r.left() + e, r.bottom())),
                egui::ResizeDirection::West,
                "rz-w",
            ),
            (
                Rect::from_min_max(Pos2::new(r.right() - e, r.top()), r.right_bottom()),
                egui::ResizeDirection::East,
                "rz-e",
            ),
            (
                Rect::from_min_max(r.left_top(), Pos2::new(r.left() + e, r.top() + e)),
                egui::ResizeDirection::NorthWest,
                "rz-nw",
            ),
            (
                Rect::from_min_max(
                    Pos2::new(r.right() - e, r.top()),
                    Pos2::new(r.right(), r.top() + e),
                ),
                egui::ResizeDirection::NorthEast,
                "rz-ne",
            ),
            (
                Rect::from_min_max(
                    Pos2::new(r.left(), r.bottom() - e),
                    Pos2::new(r.left() + e, r.bottom()),
                ),
                egui::ResizeDirection::SouthWest,
                "rz-sw",
            ),
            (
                Rect::from_min_max(Pos2::new(r.right() - e, r.bottom() - e), r.right_bottom()),
                egui::ResizeDirection::SouthEast,
                "rz-se",
            ),
        ];
        for (rect, dir, name) in zones {
            let resp = ui.interact(rect, ui.scope_id().with(name), Sense::drag());
            if resp.drag_started() {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
        }
    }

    /// 从迷你窗口还原：窗口尺寸回来，右侧面板也回到进入迷你前的状态
    fn exit_mini(&mut self, ctx: &egui::Context) {
        if let Some(open) = self.panel_before_mini.take() {
            self.panel_open = open;
        }
        self.toast = None;
        self.full_pending = true;
        ctx.request_repaint();
    }

    fn mini_ui(&mut self, root: &mut egui::Ui) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(theme::BG))
            .show(root, |ui| {
                let rect = ui.max_rect();
                let p = ui.painter().clone();
                let mac = cfg!(target_os = "macos");
                widgets::gradient_v(
                    &p,
                    rect,
                    Color32::from_rgb(0x14, 0x16, 0x1c),
                    Color32::from_rgb(0x0a, 0x0b, 0x0e),
                );

                // 迷你窗：空白处可拖动，双击还原到主窗口大小
                let drag_rect =
                    Rect::from_min_max(rect.min, Pos2::new(rect.right() - 120.0, rect.bottom()));
                let dresp = ui.interact(
                    drag_rect,
                    ui.scope_id().with("mini-drag"),
                    Sense::click_and_drag(),
                );
                if dresp.drag_started() {
                    ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                }
                if dresp.double_clicked() || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.exit_mini(ctx);
                }

                // 非 macOS：自绘关闭/最小化（macOS 用原生红绿灯）
                if !cfg!(target_os = "macos") {
                    for (name, r) in [
                        (
                            "mini-min",
                            Rect::from_min_size(
                                Pos2::new(rect.right() - 46.0, rect.top() + 7.0),
                                Vec2::splat(18.0),
                            ),
                        ),
                        (
                            "mini-close",
                            Rect::from_min_size(
                                Pos2::new(rect.right() - 24.0, rect.top() + 7.0),
                                Vec2::splat(18.0),
                            ),
                        ),
                    ] {
                        audit::ctl(name, r, ui.clip_rect());
                        let resp = ui.interact(r, ui.scope_id().with(name), Sense::click());
                        let close = name == "mini-close";
                        if resp.hovered() {
                            p.rect_filled(
                                r,
                                CornerRadius::same(4),
                                if close {
                                    Color32::from_rgb(0xc4, 0x2b, 0x1c)
                                } else {
                                    theme::white_alpha(18)
                                },
                            );
                        }
                        let c = if resp.hovered() {
                            theme::TEXT
                        } else {
                            theme::DIM
                        };
                        let ctr = r.center();
                        if close {
                            p.line_segment(
                                [ctr + Vec2::new(-3.5, -3.5), ctr + Vec2::new(3.5, 3.5)],
                                Stroke::new(1.0, c),
                            );
                            p.line_segment(
                                [ctr + Vec2::new(3.5, -3.5), ctr + Vec2::new(-3.5, 3.5)],
                                Stroke::new(1.0, c),
                            );
                        } else {
                            p.line_segment(
                                [Pos2::new(ctr.x - 4.5, ctr.y), Pos2::new(ctr.x + 4.5, ctr.y)],
                                Stroke::new(1.0, c),
                            );
                        }
                        if resp.clicked() {
                            if close {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            } else {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                            }
                        }
                    }
                }

                let snap = self.engine.snapshot();
                let station = self
                    .now
                    .as_ref()
                    .map(|n| n.station.clone())
                    .or_else(|| self.selected.as_ref().and_then(|id| self.station_by_id(id)));
                let program = self
                    .now
                    .as_ref()
                    .map(|n| n.program_name.clone())
                    .or_else(|| station.as_ref().map(|s| s.now_playing().to_string()));

                let h = rect.height();
                let bar_y = rect.bottom() - 20.0;
                let restore_rect = Rect::from_center_size(
                    Pos2::new(rect.right() - 24.0, bar_y),
                    Vec2::splat(20.0),
                );
                if let Some(st) = &station {
                    // macOS 红绿灯占据左上角，封面整体下移，避免压住按钮
                    let cover_top = rect.top() + if mac { 34.0 } else { 22.0 };
                    let cover_side = h - if mac { 64.0 } else { 52.0 };
                    let cover_rect = Rect::from_min_size(
                        Pos2::new(rect.left() + 14.0, cover_top),
                        Vec2::new(cover_side, cover_side),
                    );
                    let tex = self.images.get(ctx, &st.image);
                    widgets::cover(ui, cover_rect, 12, tex, &st.title);

                    let text_left = cover_rect.right() + 14.0;
                    let text_y = cover_rect.top() + 24.0;
                    audit::text(
                        &p,
                        Pos2::new(text_left, text_y),
                        Align2::LEFT_CENTER,
                        widgets::ellipsize(
                            ctx,
                            &st.title,
                            &theme::bold(15.0),
                            rect.width() - (text_left - rect.left()) - 150.0,
                        ),
                        theme::bold(15.0),
                        theme::TEXT,
                    );
                    if let Some(prog) = &program {
                        audit::text(
                            &p,
                            Pos2::new(text_left, text_y + 22.0),
                            Align2::LEFT_CENTER,
                            widgets::ellipsize(
                                ctx,
                                prog,
                                &theme::regular(11.5),
                                rect.width() - (text_left - rect.left()) - 150.0,
                            ),
                            theme::regular(11.5),
                            theme::WEAK,
                        );
                    }

                    // 控制
                    let play_rect = Rect::from_center_size(
                        Pos2::new(rect.right() - 96.0, cover_rect.center().y),
                        Vec2::splat(38.0),
                    );
                    let next_rect = Rect::from_center_size(
                        Pos2::new(rect.right() - 46.0, cover_rect.center().y),
                        Vec2::splat(28.0),
                    );
                    let presp = {
                        let _r = play_rect;
                        audit::ctl("mini-play", _r, ui.clip_rect());
                        ui.interact(_r, ui.scope_id().with("mini-play"), Sense::click())
                    };
                    let nresp = {
                        let _r = next_rect;
                        audit::ctl("mini-next", _r, ui.clip_rect());
                        ui.interact(_r, ui.scope_id().with("mini-next"), Sense::click())
                    };
                    p.circle_filled(
                        play_rect.center(),
                        19.0,
                        Color32::from_rgb(0xf2, 0xf3, 0xf5),
                    );
                    icons::draw(
                        &p,
                        play_rect,
                        if snap.state == PlayState::Playing {
                            Icon::Pause
                        } else {
                            Icon::Play
                        },
                        Color32::from_rgb(0x10, 0x12, 0x16),
                        20.0,
                    );
                    icons::draw(
                        &p,
                        next_rect,
                        Icon::Next,
                        if nresp.hovered() {
                            theme::TEXT
                        } else {
                            theme::WEAK
                        },
                        17.0,
                    );
                    if presp.clicked() {
                        self.toggle_play();
                    }
                    if nresp.clicked() {
                        self.step_station(1);
                    }

                    // 底部进度条（右侧留出还原按钮）
                    let bar = Rect::from_min_max(
                        Pos2::new(cover_rect.left(), bar_y - 1.5),
                        Pos2::new(restore_rect.left() - 10.0, bar_y + 1.5),
                    );
                    if let Some(now) = &self.now {
                        if now.is_replay {
                            let dur = snap.duration.unwrap_or(1.0).max(1.0);
                            widgets::progress(
                                &p,
                                bar,
                                (snap.position / dur) as f32,
                                theme::white_alpha(24),
                                theme::AMBER,
                                false,
                            );
                        } else {
                            widgets::progress(
                                &p,
                                bar,
                                1.0,
                                theme::white_alpha(24),
                                theme::accent_alpha(200),
                                false,
                            );
                        }
                    }
                } else {
                    audit::text(
                        &p,
                        rect.center(),
                        Align2::CENTER_CENTER,
                        "未在播放",
                        theme::regular(12.0),
                        theme::WEAK,
                    );
                }
                audit::ctl("mini-full", restore_rect, ui.clip_rect());
                let fresp = ui
                    .interact(
                        restore_rect,
                        ui.scope_id().with("mini-full"),
                        Sense::click(),
                    )
                    .on_hover_text("回到完整界面");
                if fresp.hovered() {
                    p.rect_filled(restore_rect, CornerRadius::same(5), theme::white_alpha(18));
                }
                icons::draw(
                    &p,
                    restore_rect.shrink(2.0),
                    Icon::Expand,
                    if fresp.hovered() {
                        theme::TEXT
                    } else {
                        theme::DIM
                    },
                    14.0,
                );
                if fresp.clicked() {
                    self.exit_mini(ctx);
                }
            });
    }
}

/// 空白区域拖动窗口；双击切换最大化（各平台一致）
fn window_drag(ui: &mut egui::Ui, rect: Rect, id: &str) {
    if rect.width() < 4.0 || rect.height() < 4.0 {
        return;
    }
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click_and_drag());
    if resp.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        let maxed = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Maximized(!maxed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(id: &str, title: &str, tag: &str, sub: &str) -> Station {
        Station {
            content_id: id.into(),
            title: title.into(),
            tag_name: tag.into(),
            subtitle: sub.into(),
            ..Default::default()
        }
    }

    #[test]
    fn enrich_uses_catalog_metadata() {
        let idx = StationIndex::build(&[st("1", "甲台", "甲市台", "正在直播：A")]);
        let out = enrich_station(st("1", "甲台(服务端)", "", "暂无节目单"), &idx);
        assert_eq!(out.title, "甲台");
        assert_eq!(out.tag_name, "甲市台");
        assert_eq!(out.subtitle, "正在直播：A");
    }

    #[test]
    fn enrich_keeps_server_row_when_unknown() {
        let idx = StationIndex::build(&[]);
        let out = enrich_station(st("9", "新台", "", ""), &idx);
        assert_eq!(out.title, "新台");
        assert_eq!(out.tag_name, "电台");
    }
}
