//! 布局自检：把所有「手工定位」的控件与文本登记下来，两两求交，重叠就报出来。
//!
//! 这类 UI 的典型 bug 是手算坐标：控件 A 从右边往左扣减宽度、控件 B 从左边往右加，
//! 两边各自算的时候都没错，碰到一起就重叠。肉眼逐个界面看太不可靠，所以让程序自己查。
//!
//! 开启方式：`--audit`（自检模式，自动轮换所有页面，结束后打印报告并退出）
//! 或环境变量 `RUST_RADIO_AUDIT=1`（正常使用，同时在终端打印重叠）。

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// 可点击/可拖动的控件
    Ctl,
    /// 绘制出来的文字
    Text,
}

struct Item {
    name: String,
    rect: Rect,
    kind: Kind,
    view: &'static str,
}

thread_local! {
    static ITEMS: std::cell::RefCell<Vec<Item>> = const { std::cell::RefCell::new(Vec::new()) };
    static ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static VIEW: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    static HITS: std::cell::RefCell<std::collections::BTreeSet<String>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
    static COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static PROBE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn set_enabled(on: bool) {
    ENABLED.with(|e| e.set(on));
}

pub fn is_enabled() -> bool {
    ENABLED.with(|e| e.get())
}

pub fn set_view(name: &str) {
    VIEW.with(|v| {
        let mut v = v.borrow_mut();
        v.clear();
        v.push_str(name);
    });
}

/// 每帧开始时清空上一帧的登记
pub fn begin_frame() {
    if !is_enabled() {
        return;
    }
    // 先审上一帧的登记（这一帧还没画），再清空
    finish_frame();
    let n = ITEMS.with(|i| i.borrow().len());
    COUNT.with(|c| {
        if n > c.get() {
            c.set(n)
        }
    });
    ITEMS.with(|i| i.borrow_mut().clear());
}

fn current_view() -> &'static str {
    // 只有少数几个 view 名字，泄漏一两个短字符串可以接受
    let name = VIEW.with(|v| v.borrow().clone());
    Box::leak(name.into_boxed_str())
}

/// 登记一个控件矩形
pub fn ctl(name: &str, rect: Rect, clip: Rect) {
    if !is_enabled() {
        return;
    }
    // 被滚动区裁掉的部分不算重叠
    let rect = rect.intersect(clip);
    if !rect.is_positive() {
        return;
    }
    ITEMS.with(|i| {
        i.borrow_mut().push(Item {
            name: name.to_owned(),
            rect,
            kind: Kind::Ctl,
            view: current_view(),
        })
    });
}

/// 画文字并登记它的实际外框（替身：`painter.text(...)`）
pub fn text(
    p: &Painter,
    pos: Pos2,
    anchor: Align2,
    text: impl ToString,
    font: FontId,
    color: Color32,
) -> Rect {
    let text = text.to_string();
    let rect = p.text(pos, anchor, text.clone(), font, color);
    let rect = rect.intersect(p.clip_rect());
    text_rect(&text, rect);
    rect
}

/// 登记一段自己排版、自己 `painter.galley(...)` 画出来的文字。
pub fn text_rect(text: &str, rect: Rect) {
    if is_enabled() && rect.is_positive() && !text.trim().is_empty() {
        ITEMS.with(|i| {
            i.borrow_mut().push(Item {
                name: format!("文本「{}」", truncate(text, 14)),
                rect,
                kind: Kind::Text,
                view: current_view(),
            })
        });
    }
}

fn truncate(s: &str, n: usize) -> String {
    let t: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        format!("{t}…")
    } else {
        t
    }
}

/// 本帧结束后检查：把新发现的重叠累积下来（跨页面去重）
pub fn finish_frame() {
    if !is_enabled() {
        return;
    }
    let mut hits: Vec<String> = Vec::new();
    ITEMS.with(|items| {
        let items = items.borrow();
        for (i, a) in items.iter().enumerate() {
            for b in items.iter().skip(i + 1) {
                let inter = a.rect.intersect(b.rect);
                if !inter.is_positive() || inter.width() < 2.0 || inter.height() < 2.0 {
                    continue;
                }
                if inter.width() * inter.height() < 16.0 {
                    continue;
                }
                if a.kind == Kind::Text && b.kind == Kind::Text {
                    continue;
                }
                if a.name.starts_with("自检") && b.name.starts_with("自检") {
                    PROBE.with(|p| p.set(true));
                    continue;
                }
                if a.name.starts_with("自检") || b.name.starts_with("自检") {
                    continue;
                }
                let (outer, inner) = if a.rect.area() >= b.rect.area() {
                    (a, b)
                } else {
                    (b, a)
                };
                // 文字在按钮里是设计如此；但两个**控件**互相包含一定是 bug
                // （例如两个按钮矩形算成了同一个位置，后注册的总是抢到点击）
                // 文字被控件包含 = 按钮里的标签，正常；
                // 两个控件互相包含 = 一定是 bug（矩形算重了）
                if outer.rect.contains_rect(inner.rect.shrink(1.0)) && inner.kind == Kind::Text {
                    continue;
                }
                hits.push(format!(
                    "[{}] {} × {}  重叠 {:.0}×{:.0}px（{} 与 {}）",
                    a.view,
                    a.name,
                    b.name,
                    inter.width(),
                    inter.height(),
                    fmt_rect(a.rect),
                    fmt_rect(b.rect)
                ));
            }
        }
    });
    if hits.is_empty() {
        return;
    }
    HITS.with(|h| {
        let mut set = h.borrow_mut();
        for line in hits {
            set.insert(line);
        }
    });
}

fn fmt_rect(r: Rect) -> String {
    format!(
        "x{:.0}..{:.0} y{:.0}..{:.0}",
        r.left(),
        r.right(),
        r.top(),
        r.bottom()
    )
}

/// 阳性对照：故意放两个部分重叠的矩形，确认检测本身有效
pub fn selftest() {
    ctl(
        "自检A",
        Rect::from_min_size(Pos2::new(100.0, 100.0), egui::Vec2::splat(40.0)),
        Rect::EVERYTHING,
    );
    ctl(
        "自检B",
        Rect::from_min_size(Pos2::new(120.0, 120.0), egui::Vec2::splat(40.0)),
        Rect::EVERYTHING,
    );
}

/// 打印累积的重叠；返回发现数量
pub fn report() -> usize {
    HITS.with(|h| {
        let h = h.borrow();
        let max = COUNT.with(|c| c.get());
        let probe = PROBE.with(|p| p.get());
        println!(
            "检测器阳性对照：{}（单帧最多登记 {max} 个矩形）",
            if probe {
                "通过"
            } else {
                "失败——检测器没在工作"
            }
        );
        if h.is_empty() {
            println!("布局自检：未发现控件重叠");
        } else {
            println!("布局自检：发现 {} 处重叠", h.len());
            for line in h.iter() {
                println!("  {line}");
            }
        }
        h.len()
    })
}
