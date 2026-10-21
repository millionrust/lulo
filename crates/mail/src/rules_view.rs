//! Mail ▸ Settings ▸ Rules: the rule list with Add Rule…, Edit… and
//! Remove, and the rule sheet laid out as Mac Mail's: Description, "If
//! any/all of the following conditions are met:", one row per condition,
//! "Perform the following actions:", one row per action, Cancel and OK.
//! Saved through `people::set_rules` (written off the UI thread).

use std::rc::Rc;

use gpui::{
    div, prelude::FluentBuilder as _, px, AppContext as _, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window,
};
use rmac_editor::InputState;
use rmac_mail::rules::{Action, Condition, Field, MailboxRef, Operator, Rule};
use rmac_mail::FlagColour;
use rmac_ui::{mac, Button, Checkbox, PopUpButton, PopupMenuItem, StyledExt as _, TextField};

use crate::people;

type Choose = Rc<dyn Fn(&mut RulesPane, &mut Window, &mut Context<RulesPane>)>;

struct Sheet {
    /// The rule being edited, or `None` for Add Rule….
    index: Option<usize>,
    rule: Rule,
    description: Entity<InputState>,
    /// One text field per condition, in step with `rule.conditions`.
    values: Vec<Entity<InputState>>,
}

pub struct RulesPane {
    rules: Vec<Rule>,
    selected: Option<usize>,
    sheet: Option<Sheet>,
}

impl RulesPane {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            rules: people::rules(cx).as_ref().clone(),
            selected: None,
            sheet: None,
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        people::set_rules(self.rules.clone(), cx);
        cx.notify();
    }

    fn text_input(value: &str, window: &mut Window, cx: &mut Context<Self>) -> Entity<InputState> {
        let value = value.to_owned();
        cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(value, window, cx);
            state
        })
    }

    fn open_sheet(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let rule = index
            .and_then(|index| self.rules.get(index).cloned())
            .unwrap_or_else(|| Rule {
                description: format!("Rule #{}", self.rules.len() + 1),
                ..Rule::default()
            });
        let description = Self::text_input(&rule.description, window, cx);
        let values = rule
            .conditions
            .iter()
            .map(|condition| Self::text_input(&condition.value, window, cx))
            .collect();
        self.sheet = Some(Sheet {
            index,
            rule,
            description,
            values,
        });
        cx.notify();
    }

    fn commit_sheet(&mut self, cx: &mut Context<Self>) {
        let Some(sheet) = self.sheet.take() else {
            return;
        };
        let mut rule = sheet.rule;
        rule.description = sheet.description.read(cx).value().trim().to_owned();
        for (condition, value) in rule.conditions.iter_mut().zip(&sheet.values) {
            condition.value = value.read(cx).value().to_string();
        }
        match sheet.index {
            Some(index) if index < self.rules.len() => self.rules[index] = rule,
            _ => {
                self.rules.push(rule);
                self.selected = Some(self.rules.len() - 1);
            }
        }
        self.save(cx);
    }

    fn sheet_mut(&mut self) -> Option<&mut Sheet> {
        self.sheet.as_mut()
    }

    fn popup(
        &self,
        id: impl Into<gpui::ElementId>,
        label: impl Into<SharedString>,
        width: f32,
        choices: Vec<(String, bool, Choose)>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity();
        PopUpButton::new(id, label)
            .w(px(width))
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(menu, |menu, (label, checked, choose)| {
                    let view = view.clone();
                    let choose = choose.clone();
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(*checked)
                            .on_click(move |_, window, cx| {
                                view.update(cx, |this, cx| {
                                    choose(this, window, cx);
                                    cx.notify();
                                });
                            }),
                    )
                })
            })
    }

    fn list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("mail-rules-list")
            .role(Role::List)
            .aria_label("Rules")
            .h(px(260.0))
            .overflow_y_scroll()
            .rounded(px(mac::radius_card()))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::control_fill())
            .children(self.rules.iter().enumerate().map(|(index, rule)| {
                let selected = self.selected == Some(index);
                div()
                    .id(("mail-rule", index))
                    .role(Role::ListItem)
                    .aria_label(rule.description.clone())
                    .aria_selected(selected)
                    .h(px(26.0))
                    .px(px(8.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .when(selected, |row| row.bg(mac::accent_subtle()))
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            this.selected = Some(index);
                            if event.click_count() >= 2 {
                                this.open_sheet(Some(index), window, cx);
                            }
                            cx.notify();
                        }),
                    )
                    .child(
                        // The Active box carries the rule's name, so a screen
                        // reader reads "Rule #1, checked".
                        Checkbox::new(("mail-rule-active", index))
                            .label(rule.description.clone())
                            .checked(rule.enabled)
                            .on_change(cx.listener(move |this, value: &bool, _, cx| {
                                if let Some(rule) = this.rules.get_mut(index) {
                                    rule.enabled = *value;
                                }
                                this.save(cx);
                            })),
                    )
            }))
            .when(self.rules.is_empty(), |list| {
                list.child(
                    div()
                        .p(px(10.0))
                        .text_size(px(12.0))
                        .text_color(mac::text_secondary())
                        .child("No rules. Rules act on new messages as they arrive in an Inbox."),
                )
            })
    }

    fn buttons(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_selection = self.selected.is_some_and(|index| index < self.rules.len());
        div()
            .flex()
            .gap(px(8.0))
            .child(
                Button::new("mail-rules-add", "Add Rule…")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.open_sheet(None, window, cx))),
            )
            .child(
                Button::new("mail-rules-edit", "Edit…")
                    .small()
                    .disabled(!has_selection)
                    .on_click(cx.listener(|this, _, window, cx| {
                        let index = this.selected;
                        this.open_sheet(index, window, cx);
                    })),
            )
            .child(
                Button::new("mail-rules-duplicate", "Duplicate")
                    .small()
                    .disabled(!has_selection)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(rule) = this.selected.and_then(|index| this.rules.get(index)) {
                            let mut copy = rule.clone();
                            copy.description = format!("{} Copy", copy.description);
                            this.rules.push(copy);
                            this.selected = Some(this.rules.len() - 1);
                            this.save(cx);
                        }
                    })),
            )
            .child(
                Button::new("mail-rules-remove", "Remove")
                    .small()
                    .disabled(!has_selection)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(index) = this.selected.take() {
                            if index < this.rules.len() {
                                this.rules.remove(index);
                                this.save(cx);
                            }
                        }
                    })),
            )
    }

    fn condition_row(
        &self,
        index: usize,
        condition: &Condition,
        value: Option<&Entity<InputState>>,
        last: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let fields = Field::ALL
            .into_iter()
            .map(|field| {
                let choose: Choose = Rc::new(move |this: &mut RulesPane, _, _| {
                    if let Some(sheet) = this.sheet_mut() {
                        sheet.rule.conditions[index].field = field;
                    }
                });
                (field.label().to_owned(), field == condition.field, choose)
            })
            .collect();
        let operators = Operator::ALL
            .into_iter()
            .map(|operator| {
                let choose: Choose = Rc::new(move |this: &mut RulesPane, _, _| {
                    if let Some(sheet) = this.sheet_mut() {
                        sheet.rule.conditions[index].operator = operator;
                    }
                });
                (
                    operator.label().to_owned(),
                    operator == condition.operator,
                    choose,
                )
            })
            .collect();
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(self.popup(
                ("mail-rule-field", index),
                condition.field.label(),
                190.0,
                fields,
                cx,
            ))
            .when(condition.field.takes_text(), |row| {
                row.child(self.popup(
                    ("mail-rule-operator", index),
                    condition.operator.label(),
                    140.0,
                    operators,
                    cx,
                ))
                .when_some(value.cloned(), |row, value| {
                    row.child(
                        div()
                            .id(("mail-rule-value", index))
                            .w(px(180.0))
                            .h(px(24.0))
                            .rounded(px(mac::radius_control()))
                            .bg(mac::window())
                            .border_1()
                            .border_color(mac::separator())
                            .child(TextField::new(&value).appearance(false).small()),
                    )
                })
            })
            .child(
                Button::new(("mail-rule-condition-remove", index), "−")
                    .small()
                    .disabled(index == 0 && last)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(sheet) = this.sheet_mut() {
                            if sheet.rule.conditions.len() > 1 {
                                sheet.rule.conditions.remove(index);
                                sheet.values.remove(index);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("mail-rule-condition-add", index), "+")
                    .small()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let input = Self::text_input("", window, cx);
                        if let Some(sheet) = this.sheet_mut() {
                            sheet
                                .rule
                                .conditions
                                .insert(index + 1, Condition::default());
                            sheet.values.insert(index + 1, input);
                        }
                        cx.notify();
                    })),
            )
    }

    fn action_row(
        &self,
        index: usize,
        action: &Action,
        last: bool,
        destinations: &[(String, String)],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let kinds = Action::kinds()
            .into_iter()
            .map(|kind| {
                let checked = kind.same_kind(action);
                let label = kind.label().to_owned();
                let choose: Choose = Rc::new(move |this: &mut RulesPane, _, _| {
                    if let Some(sheet) = this.sheet_mut() {
                        if !sheet.rule.actions[index].same_kind(&kind) {
                            sheet.rule.actions[index] = kind.clone();
                        }
                    }
                });
                (label, checked, choose)
            })
            .collect();
        let detail = match action {
            Action::MoveMessage { to } | Action::CopyMessage { to } => {
                let label = if to.name.is_empty() {
                    "Choose Mailbox".to_owned()
                } else {
                    format!("{} — {}", to.account, to.name)
                };
                let choices = destinations
                    .iter()
                    .map(|(account, name)| {
                        let target = MailboxRef {
                            account: account.clone(),
                            name: name.clone(),
                        };
                        let checked = *to == target;
                        let choose: Choose = Rc::new(move |this: &mut RulesPane, _, _| {
                            if let Some(sheet) = this.sheet_mut() {
                                match &mut sheet.rule.actions[index] {
                                    Action::MoveMessage { to } | Action::CopyMessage { to } => {
                                        *to = target.clone();
                                    }
                                    _ => {}
                                }
                            }
                        });
                        (format!("{account} — {name}"), checked, choose)
                    })
                    .collect();
                Some((label, choices))
            }
            Action::MarkAsFlagged { .. } => {
                let current = action.flag_colour().unwrap_or_default();
                let choices = FlagColour::MENU
                    .into_iter()
                    .map(|colour| {
                        let choose: Choose = Rc::new(move |this: &mut RulesPane, _, _| {
                            if let Some(sheet) = this.sheet_mut() {
                                sheet.rule.actions[index] = Action::MarkAsFlagged {
                                    colour: colour.label().to_owned(),
                                };
                            }
                        });
                        (colour.label().to_owned(), colour == current, choose)
                    })
                    .collect();
                Some((current.label().to_owned(), choices))
            }
            _ => None,
        };
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(self.popup(
                ("mail-rule-action", index),
                action.label(),
                190.0,
                kinds,
                cx,
            ))
            .when_some(detail, |row, (label, choices)| {
                row.child(self.popup(
                    ("mail-rule-action-detail", index),
                    label,
                    260.0,
                    choices,
                    cx,
                ))
            })
            .child(
                Button::new(("mail-rule-action-remove", index), "−")
                    .small()
                    .disabled(index == 0 && last)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(sheet) = this.sheet_mut() {
                            if sheet.rule.actions.len() > 1 {
                                sheet.rule.actions.remove(index);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new(("mail-rule-action-add", index), "+")
                    .small()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(sheet) = this.sheet_mut() {
                            sheet.rule.actions.insert(index + 1, Action::MarkAsRead);
                        }
                        cx.notify();
                    })),
            )
    }

    fn sheet_view(&self, sheet: &Sheet, cx: &mut Context<Self>) -> impl IntoElement {
        let destinations = crate::view::rule_destinations(cx);
        let rule = &sheet.rule;
        let any_all: Vec<(String, bool, Choose)> = [(true, "any"), (false, "all")]
            .into_iter()
            .map(|(any, label)| {
                let choose: Choose = Rc::new(move |this: &mut RulesPane, _, _| {
                    if let Some(sheet) = this.sheet_mut() {
                        sheet.rule.any = any;
                    }
                });
                (label.to_owned(), rule.any == any, choose)
            })
            .collect();
        let conditions = rule.conditions.len();
        let actions = rule.actions.len();
        div()
            .id("mail-rule-sheet")
            .role(Role::Dialog)
            .aria_label("Rule")
            .p(px(16.0))
            .rounded(px(mac::radius_card()))
            .border_1()
            .border_color(mac::separator())
            .bg(mac::window())
            .v_flex()
            .gap(px(10.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().text_size(px(13.0)).child("Description:"))
                    .child(
                        div()
                            .id("mail-rule-description")
                            .w(px(360.0))
                            .h(px(24.0))
                            .rounded(px(mac::radius_control()))
                            .border_1()
                            .border_color(mac::separator())
                            .child(TextField::new(&sheet.description).appearance(false).small()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .text_size(px(13.0))
                    .child("If")
                    .child(self.popup(
                        "mail-rule-any-all",
                        if rule.any { "any" } else { "all" },
                        70.0,
                        any_all,
                        cx,
                    ))
                    .child("of the following conditions are met:"),
            )
            .children(
                rule.conditions
                    .iter()
                    .enumerate()
                    .map(|(index, condition)| {
                        self.condition_row(
                            index,
                            condition,
                            sheet.values.get(index),
                            conditions == 1,
                            cx,
                        )
                    }),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .child("Perform the following actions:"),
            )
            .children(rule.actions.iter().enumerate().map(|(index, action)| {
                self.action_row(index, action, actions == 1, &destinations, cx)
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new("mail-rule-cancel", "Cancel")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sheet = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("mail-rule-ok", "OK")
                            .small()
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| this.commit_sheet(cx))),
                    ),
            )
    }
}

impl Render for RulesPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet = self.sheet.take();
        let body = div().v_flex().gap(px(10.0)).map(|body| match &sheet {
            Some(sheet) => body.child(self.sheet_view(sheet, cx)),
            None => body
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(13.0))
                        .child("Rules"),
                )
                .child(self.list(cx))
                .child(self.buttons(cx)),
        });
        self.sheet = sheet;
        body
    }
}
