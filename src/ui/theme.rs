//! Theme settings and shared translucent surfaces for the native workbench.
#[cfg(all(test, windows))]
#[path = "theme_native_test.rs"]
mod native_test;
use super::*;
use vibe_terminal::model::WindowMaterial;

impl Ui {
    pub(super) fn surface_background(&self, color: u32) -> Hsla {
        crate::material::surface(self.material_appearance, color)
    }
    pub(super) fn set_window_material(
        &mut self,
        material: WindowMaterial,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.material_appearance = crate::material::apply(material, window, cx);
        self.session.data.window_material = material;
        self.session.persist();
        cx.notify();
    }
    pub(super) fn theme_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.session.data.window_material;
        let mut pane = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(div().text_sm().child("窗口材质"))
            .child(
                div()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("保持深色外观，选择 Windows 原生背景材质。切换立即生效并自动保存。"),
            );
        for (id, title, description, material) in [
            (
                "theme-mica",
                "Mica · 云母",
                "柔和融合桌面壁纸色彩，适合长时间阅读。",
                WindowMaterial::Mica,
            ),
            (
                "theme-mica-alt",
                "Mica Alt · 云母变体",
                "更明显的壁纸底色与层次。",
                WindowMaterial::MicaAlt,
            ),
            (
                "theme-acrylic",
                "Acrylic · 亚克力",
                "模糊透出窗口后方内容，呈现半透明毛玻璃效果。",
                WindowMaterial::Acrylic,
            ),
        ] {
            pane = pane.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        Button::new(id)
                            .label(title)
                            .selected(selected == material)
                            .custom(Self::button_style(selected == material, cx))
                            .on_click(cx.listener(move |ui, _, window, cx| {
                                ui.set_window_material(material, window, cx)
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(rgb(MUTED))
                            .child(description),
                    ),
            );
        }
        if crate::material::windows_build() < 22621 {
            pane = pane.child(div().text_sm().text_color(rgb(MUTED)).child(
                if crate::material::windows_build() >= 17763 {
                    "当前系统不支持云母材质，Mica 和 Mica Alt 将使用 Acrylic 效果。"
                } else {
                    "当前系统不支持这些原生材质，将使用不透明深色背景。"
                },
            ));
        }
        pane.into_any_element()
    }
}
