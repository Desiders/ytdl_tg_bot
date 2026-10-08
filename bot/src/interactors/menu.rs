use std::sync::Arc;

use rust_i18n::t;
use telers::{
    callback_data::CallbackData,
    errors::HandlerError,
    fsm::{Context, MemoryStorage},
    utils::text::{html_code, html_expandable_blockquote, html_quote, html_text_link},
};
use tracing::error;

use crate::{
    config::Config,
    entities::{ChatConfig, ChatConfigExcludeDomain, ChatConfigExcludeDomains, ChatConfigUpdate},
    interactors::{config::MAX_EXCLUDE_DOMAINS, stats, Interactor},
    locale::Locale,
    services::{
        chat,
        messenger::{AnswerCallbackRequest, Button, ButtonAction, EditMenuRequest, Keyboard, MessengerPort, SendMenuRequest},
    },
    utils::ErrorFormatter,
    value_objects::menu::{DeleteDomain, DomainKey, MenuState, OpenScreen, Screen, SetLanguage, SetLinkVisibility},
};

const LOCALES: [Locale; 3] = [Locale::En, Locale::Ru, Locale::Uk];

pub type Fsm = Context<MemoryStorage>;

pub enum MenuTarget<'a> {
    Send {
        chat_id: i64,
    },
    Edit {
        chat_id: i64,
        message_id: i64,
        callback_id: &'a str,
    },
}

pub struct OpenMenu<Messenger> {
    cfg: Arc<Config>,
    error_formatter: Arc<ErrorFormatter>,
    messenger: Arc<Messenger>,
    stats: Arc<stats::Stats<Messenger>>,
}

impl<Messenger> OpenMenu<Messenger> {
    #[must_use]
    pub const fn new(
        cfg: Arc<Config>,
        error_formatter: Arc<ErrorFormatter>,
        messenger: Arc<Messenger>,
        stats: Arc<stats::Stats<Messenger>>,
    ) -> Self {
        Self {
            cfg,
            error_formatter,
            messenger,
            stats,
        }
    }
}

pub struct Notice {
    pub text: String,
    pub details: Option<String>,
}

impl Notice {
    #[must_use]
    pub const fn new(text: String) -> Self {
        Self { text, details: None }
    }
}

pub struct OpenMenuInput<'a> {
    pub target: MenuTarget<'a>,
    pub screen: Screen,
    pub notice: Option<Notice>,
    pub chat_cfg: &'a ChatConfig,
    pub exclude_domains: &'a ChatConfigExcludeDomains,
    pub fsm: &'a Fsm,
}

impl<Messenger> Interactor<OpenMenuInput<'_>> for &OpenMenu<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: OpenMenuInput<'_>) -> Result<Self::Output, Self::Err> {
        let domains = &input.exclude_domains.0;
        let screen = existing_screen(input.screen, domains);
        track_input(screen, input.fsm).await;

        let locale = input.chat_cfg.locale();
        let page = Page {
            locale,
            link_is_visible: input.chat_cfg.link_is_visible,
            domains,
            max_file_size_in_mb: self.cfg.yt_dlp.max_file_size / 1000 / 1000,
            src_url: &self.cfg.bot.src_url,
            username: self.username(screen).await,
            stats: self.stats_text(screen, locale).await,
        };
        let view = render(screen, &page);

        match input.target {
            MenuTarget::Send { chat_id } => self.send(chat_id, view, input.notice).await,
            MenuTarget::Edit {
                chat_id,
                message_id,
                callback_id,
            } => self.edit(chat_id, message_id, callback_id, view, input.notice).await,
        }

        Ok(())
    }
}

impl<Messenger> OpenMenu<Messenger>
where
    Messenger: MessengerPort,
{
    async fn send(&self, chat_id: i64, view: View, notice: Option<Notice>) {
        // A new message has no callback answer to carry the notice, so the notice heads the text.
        let text = with_notice(notice, view.text);
        if let Err(err) = self
            .messenger
            .send_menu(SendMenuRequest {
                chat_id,
                text: &text,
                keyboard: &view.keyboard,
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Send menu error");
        }
    }

    async fn edit(&self, chat_id: i64, message_id: i64, callback_id: &str, view: View, notice: Option<Notice>) {
        // The edit may wait out a Telegram cooldown, and an answer that comes too late is rejected, so answer first.
        //  https://github.com/tdlib/telegram-bot-api/blob/e3e9dd8e5b3d7ab8537cd5a10dc31d5ffa8f82d1/telegram-bot-api/Client.cpp#L150
        if let Err(err) = self
            .messenger
            .answer_callback(AnswerCallbackRequest {
                callback_id,
                text: notice.as_ref().map(|notice| notice.text.as_str()),
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Answer callback error");
        }

        if let Err(err) = self
            .messenger
            .edit_menu(EditMenuRequest {
                chat_id,
                message_id,
                text: &view.text,
                keyboard: &view.keyboard,
            })
            .await
        {
            error!(err = %self.error_formatter.format(&err), "Edit menu error");
        }
    }

    async fn username(&self, screen: Screen) -> String {
        if screen != Screen::HelpInline {
            return String::new();
        }
        match self.messenger.username().await {
            Ok(username) => username,
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Get messenger username error");
                String::new()
            }
        }
    }

    async fn stats_text(&self, screen: Screen, locale: Locale) -> String {
        if screen == Screen::Stats {
            self.stats.text(locale.as_str()).await
        } else {
            String::new()
        }
    }
}

pub struct SetMenuLanguage<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    update_chat_cfg: Arc<chat::UpdateChatConfig>,
    open_menu: Arc<OpenMenu<Messenger>>,
}

impl<Messenger> SetMenuLanguage<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        update_chat_cfg: Arc<chat::UpdateChatConfig>,
        open_menu: Arc<OpenMenu<Messenger>>,
    ) -> Self {
        Self {
            error_formatter,
            update_chat_cfg,
            open_menu,
        }
    }
}

pub struct SetMenuLanguageInput<'a> {
    pub target: MenuTarget<'a>,
    pub locale: Locale,
    pub chat_cfg: &'a ChatConfig,
    pub exclude_domains: &'a ChatConfigExcludeDomains,
    pub fsm: &'a Fsm,
}

impl<Messenger> Interactor<SetMenuLanguageInput<'_>> for &SetMenuLanguage<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: SetMenuLanguageInput<'_>) -> Result<Self::Output, Self::Err> {
        let (chat_cfg, notice) = match self
            .update_chat_cfg
            .execute(chat::UpdateChatConfigInput {
                dto: ChatConfigUpdate::new(input.chat_cfg.tg_id).with_language(input.locale.as_str().to_owned()),
            })
            .await
        {
            Ok(chat_cfg) => (chat_cfg, None),
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Update error");
                let notice = t!("lang.error", locale = input.chat_cfg.locale().as_str()).into_owned();
                (input.chat_cfg.clone(), Some(Notice::new(notice)))
            }
        };

        self.open_menu
            .as_ref()
            .execute(OpenMenuInput {
                target: input.target,
                screen: Screen::Language,
                notice,
                chat_cfg: &chat_cfg,
                exclude_domains: input.exclude_domains,
                fsm: input.fsm,
            })
            .await
    }
}

pub struct SetMenuLinkVisibility<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    update_chat_cfg: Arc<chat::UpdateChatConfig>,
    open_menu: Arc<OpenMenu<Messenger>>,
}

impl<Messenger> SetMenuLinkVisibility<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        update_chat_cfg: Arc<chat::UpdateChatConfig>,
        open_menu: Arc<OpenMenu<Messenger>>,
    ) -> Self {
        Self {
            error_formatter,
            update_chat_cfg,
            open_menu,
        }
    }
}

pub struct SetMenuLinkVisibilityInput<'a> {
    pub target: MenuTarget<'a>,
    pub link_is_visible: bool,
    pub chat_cfg: &'a ChatConfig,
    pub exclude_domains: &'a ChatConfigExcludeDomains,
    pub fsm: &'a Fsm,
}

impl<Messenger> Interactor<SetMenuLinkVisibilityInput<'_>> for &SetMenuLinkVisibility<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: SetMenuLinkVisibilityInput<'_>) -> Result<Self::Output, Self::Err> {
        let (chat_cfg, notice) = match self
            .update_chat_cfg
            .execute(chat::UpdateChatConfigInput {
                dto: ChatConfigUpdate::new(input.chat_cfg.tg_id).with_link_is_visible(input.link_is_visible),
            })
            .await
        {
            Ok(chat_cfg) => (chat_cfg, None),
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Update error");
                let notice = t!("link_visibility.error", locale = input.chat_cfg.locale().as_str()).into_owned();
                (input.chat_cfg.clone(), Some(Notice::new(notice)))
            }
        };

        self.open_menu
            .as_ref()
            .execute(OpenMenuInput {
                target: input.target,
                screen: Screen::Settings,
                notice,
                chat_cfg: &chat_cfg,
                exclude_domains: input.exclude_domains,
                fsm: input.fsm,
            })
            .await
    }
}

pub struct AddMenuDomain<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    add_domain: Arc<chat::AddExcludeDomain>,
    open_menu: Arc<OpenMenu<Messenger>>,
}

impl<Messenger> AddMenuDomain<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        add_domain: Arc<chat::AddExcludeDomain>,
        open_menu: Arc<OpenMenu<Messenger>>,
    ) -> Self {
        Self {
            error_formatter,
            add_domain,
            open_menu,
        }
    }
}

pub struct AddMenuDomainInput<'a> {
    pub chat_id: i64,
    pub host: Option<&'a str>,
    pub chat_cfg: &'a ChatConfig,
    pub exclude_domains: &'a ChatConfigExcludeDomains,
    pub fsm: &'a Fsm,
}

impl<Messenger> Interactor<AddMenuDomainInput<'_>> for &AddMenuDomain<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: AddMenuDomainInput<'_>) -> Result<Self::Output, Self::Err> {
        let locale = input.chat_cfg.locale().as_str();
        let mut domains = input.exclude_domains.0.clone();
        let (screen, notice) = match input.host {
            None => (
                Screen::DomainAdd,
                Notice::new(t!("menu.domain_invalid", locale = locale).into_owned()),
            ),
            Some(host) if domains.iter().any(|domain| domain == host) => (
                Screen::Domains,
                Notice::new(t!("exclude_domain.already_in_list", locale = locale).into_owned()),
            ),
            Some(_) if domains.len() >= MAX_EXCLUDE_DOMAINS => (
                Screen::Domains,
                Notice::new(t!("exclude_domain.limit_reached", locale = locale).into_owned()),
            ),
            Some(host) => (Screen::Domains, self.add(input.chat_cfg, &mut domains, host).await),
        };

        self.open_menu
            .as_ref()
            .execute(OpenMenuInput {
                target: MenuTarget::Send { chat_id: input.chat_id },
                screen,
                notice: Some(notice),
                chat_cfg: input.chat_cfg,
                exclude_domains: &ChatConfigExcludeDomains(domains),
                fsm: input.fsm,
            })
            .await
    }
}

impl<Messenger> AddMenuDomain<Messenger> {
    async fn add(&self, chat_cfg: &ChatConfig, domains: &mut Vec<String>, host: &str) -> Notice {
        let locale = chat_cfg.locale().as_str();
        match self
            .add_domain
            .execute(chat::ExcludeDomainInput {
                dto: ChatConfigExcludeDomain::new(chat_cfg.tg_id, host.to_owned()),
            })
            .await
        {
            Ok(()) => {
                domains.push(host.to_owned());
                Notice::new(t!("menu.domain_added", locale = locale, domain = host).into_owned())
            }
            Err(err) => {
                let details = self.error_formatter.format(&err).into_owned();
                error!(err = %details, "Add error");
                Notice {
                    text: t!("exclude_domain.add_error", locale = locale).into_owned(),
                    details: Some(details),
                }
            }
        }
    }
}

pub struct RemoveMenuDomain<Messenger> {
    error_formatter: Arc<ErrorFormatter>,
    remove_domain: Arc<chat::RemoveExcludeDomain>,
    open_menu: Arc<OpenMenu<Messenger>>,
}

impl<Messenger> RemoveMenuDomain<Messenger> {
    #[must_use]
    pub const fn new(
        error_formatter: Arc<ErrorFormatter>,
        remove_domain: Arc<chat::RemoveExcludeDomain>,
        open_menu: Arc<OpenMenu<Messenger>>,
    ) -> Self {
        Self {
            error_formatter,
            remove_domain,
            open_menu,
        }
    }
}

pub struct RemoveMenuDomainInput<'a> {
    pub target: MenuTarget<'a>,
    pub domain: DomainKey,
    pub chat_cfg: &'a ChatConfig,
    pub exclude_domains: &'a ChatConfigExcludeDomains,
    pub fsm: &'a Fsm,
}

impl<Messenger> Interactor<RemoveMenuDomainInput<'_>> for &RemoveMenuDomain<Messenger>
where
    Messenger: MessengerPort,
{
    type Output = ();
    type Err = HandlerError;

    async fn execute(self, input: RemoveMenuDomainInput<'_>) -> Result<Self::Output, Self::Err> {
        let mut domains = input.exclude_domains.0.clone();
        let notice = self.remove(input.chat_cfg, &mut domains, input.domain).await;

        self.open_menu
            .as_ref()
            .execute(OpenMenuInput {
                target: input.target,
                screen: Screen::Domains,
                notice,
                chat_cfg: input.chat_cfg,
                exclude_domains: &ChatConfigExcludeDomains(domains),
                fsm: input.fsm,
            })
            .await
    }
}

impl<Messenger> RemoveMenuDomain<Messenger> {
    async fn remove(&self, chat_cfg: &ChatConfig, domains: &mut Vec<String>, domain_key: DomainKey) -> Option<Notice> {
        let locale = chat_cfg.locale().as_str();
        let index = domain_key.position(domains)?;
        let domain = domains[index].clone();
        match self
            .remove_domain
            .execute(chat::ExcludeDomainInput {
                dto: ChatConfigExcludeDomain::new(chat_cfg.tg_id, domain.clone()),
            })
            .await
        {
            Ok(()) => {
                domains.remove(index);
                Some(Notice::new(
                    t!("menu.domain_removed", locale = locale, domain = domain).into_owned(),
                ))
            }
            Err(err) => {
                error!(err = %self.error_formatter.format(&err), "Remove error");
                Some(Notice::new(t!("exclude_domain.remove_error", locale = locale).into_owned()))
            }
        }
    }
}

// Only the add-domain prompt waits for typed input; every other screen cancels it. Finishing first keeps the state
//  stack at one entry, since `set_state` pushes onto it.
//  https://github.com/Desiders/telers/blob/a14a34520eaeffccd1ebdbb4fffe8e44ead81bb9/telers/src/fsm/storage/memory.rs#L47-L57
async fn track_input(screen: Screen, fsm: &Fsm) {
    let result = match fsm.finish().await {
        Ok(()) if screen == Screen::DomainAdd => fsm.set_state(MenuState::DomainInput).await,
        result => result,
    };
    if let Err(err) = result {
        error!(%err, "Menu state error");
    }
}

fn with_notice(notice: Option<Notice>, text: String) -> String {
    let Some(notice) = notice else {
        return text;
    };
    match notice.details {
        Some(details) => format!(
            "{}\n{}\n\n{text}",
            html_quote(notice.text),
            html_expandable_blockquote(html_quote(details))
        ),
        None => format!("{}\n\n{text}", html_quote(notice.text)),
    }
}

// A domain screen whose domain left the list, after a removal elsewhere, falls back to the list.
fn existing_screen(screen: Screen, domains: &[String]) -> Screen {
    match screen {
        Screen::Domain { key } | Screen::DomainDelete { key } if key.position(domains).is_none() => Screen::Domains,
        screen => screen,
    }
}

struct Page<'a> {
    locale: Locale,
    link_is_visible: bool,
    domains: &'a [String],
    max_file_size_in_mb: u64,
    src_url: &'a str,
    username: String,
    stats: String,
}

struct View {
    text: String,
    keyboard: Keyboard,
}

#[allow(clippy::too_many_lines)]
fn render(screen: Screen, page: &Page<'_>) -> View {
    let locale = page.locale.as_str();
    let label = |key: &str| t!(key, locale = locale).into_owned();
    let back = |to: Screen| open(label("menu.btn_back"), to);

    let (text, buttons) = match screen {
        Screen::Main => (
            t!("menu.main", locale = locale, max_file_size_in_mb = page.max_file_size_in_mb).into_owned(),
            vec![
                open(label("menu.btn_settings"), Screen::Settings),
                open(label("menu.btn_language"), Screen::Language),
                open(label("menu.btn_help"), Screen::Help),
                open(label("menu.btn_stats"), Screen::Stats),
                Button {
                    text: label("menu.btn_source"),
                    action: ButtonAction::Url(page.src_url.to_owned()),
                },
            ],
        ),
        Screen::Settings => (
            label("menu.settings"),
            vec![
                Button {
                    text: with_tick(&label("menu.btn_link_visibility"), page.link_is_visible),
                    action: callback(&SetLinkVisibility {
                        visible: !page.link_is_visible,
                    }),
                },
                open(label("menu.btn_domains"), Screen::Domains),
                back(Screen::Main),
            ],
        ),
        Screen::Language => (
            label("menu.language"),
            LOCALES
                .into_iter()
                .map(|target| Button {
                    text: with_selection(&t!("lang.name", locale = target.as_str()), target == page.locale),
                    action: callback(&SetLanguage { locale: target }),
                })
                .chain([back(Screen::Main)])
                .collect(),
        ),
        Screen::Help => (
            label("menu.help"),
            vec![
                open(label("menu.btn_help_commands"), Screen::HelpCommands),
                open(label("menu.btn_help_arguments"), Screen::HelpArguments),
                open(label("menu.btn_help_inline"), Screen::HelpInline),
                back(Screen::Main),
            ],
        ),
        Screen::HelpCommands => (label("help.commands"), vec![back(Screen::Help)]),
        Screen::HelpArguments => {
            let source_code = html_text_link(label("start.source_code_label"), html_quote(page.src_url));
            let text = t!(
                "help.arguments",
                locale = locale,
                max_file_size_in_mb = page.max_file_size_in_mb,
                source_code = source_code,
            )
            .into_owned();
            (text, vec![back(Screen::Help)])
        }
        Screen::HelpInline => (
            t!("help.inline", locale = locale, username = page.username).into_owned(),
            vec![back(Screen::Help)],
        ),
        Screen::Stats => (
            page.stats.clone(),
            vec![open(label("menu.btn_refresh"), Screen::Stats), back(Screen::Main)],
        ),
        Screen::Domains => {
            let text = t!(
                "menu.domains",
                locale = locale,
                count = page.domains.len(),
                limit = MAX_EXCLUDE_DOMAINS,
            )
            .into_owned();
            let domains = page.domains.iter().map(|domain| {
                open(
                    domain.clone(),
                    Screen::Domain {
                        key: DomainKey::new(domain),
                    },
                )
            });
            let add = (page.domains.len() < MAX_EXCLUDE_DOMAINS).then(|| open(label("menu.btn_add"), Screen::DomainAdd));
            (text, domains.chain(add).chain([back(Screen::Settings)]).collect())
        }
        Screen::DomainAdd => (label("menu.domain_add"), vec![open(label("menu.btn_cancel"), Screen::Domains)]),
        Screen::Domain { key } => (
            t!("menu.domain", locale = locale, domain = domain_code(page, key)).into_owned(),
            vec![open(label("menu.btn_delete"), Screen::DomainDelete { key }), back(Screen::Domains)],
        ),
        Screen::DomainDelete { key } => (
            t!("menu.domain_delete", locale = locale, domain = domain_code(page, key)).into_owned(),
            vec![
                Button {
                    text: label("menu.btn_confirm_delete"),
                    action: callback(&DeleteDomain { domain: key }),
                },
                back(Screen::Domain { key }),
            ],
        ),
    };

    View {
        text,
        keyboard: Keyboard {
            rows: buttons.into_iter().map(|button| vec![button]).collect(),
        },
    }
}

fn open(text: String, screen: Screen) -> Button {
    Button {
        text,
        action: callback(&OpenScreen { screen }),
    }
}

// Every callback here is a short prefix and a short value, far under the 64-byte limit.
fn callback(data: &impl CallbackData) -> ButtonAction {
    ButtonAction::Callback(data.pack().expect("Menu callback data fits the limit"))
}

fn with_tick(text: &str, value: bool) -> String {
    format!("{text} [{}]", if value { "✅" } else { "❌" })
}

fn with_selection(text: &str, selected: bool) -> String {
    if selected {
        format!("{text} [✅]")
    } else {
        text.to_owned()
    }
}

fn domain_code(page: &Page<'_>, domain_key: DomainKey) -> String {
    domain_key
        .position(page.domains)
        .map(|index| html_code(html_quote(&page.domains[index])))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(domains: &[String]) -> Page<'_> {
        Page {
            locale: Locale::En,
            link_is_visible: true,
            domains,
            max_file_size_in_mb: 50,
            src_url: "https://example.com/source",
            username: "test_bot".to_owned(),
            stats: "stats".to_owned(),
        }
    }

    fn callbacks(view: &View) -> Vec<&str> {
        view.keyboard
            .rows
            .iter()
            .flatten()
            .filter_map(|button| match &button.action {
                ButtonAction::Callback(data) => Some(data.as_str()),
                ButtonAction::Url(_) => None,
            })
            .collect()
    }

    fn domains(count: usize) -> Vec<String> {
        (0..count).map(|index| format!("site{index}.example")).collect()
    }

    #[test]
    fn every_screen_has_one_button_per_row() {
        let domains = domains(2);
        for screen in [
            Screen::Main,
            Screen::Settings,
            Screen::Language,
            Screen::Help,
            Screen::HelpCommands,
            Screen::HelpArguments,
            Screen::HelpInline,
            Screen::Stats,
            Screen::Domains,
            Screen::DomainAdd,
            Screen::Domain {
                key: DomainKey::new(&domains[1]),
            },
            Screen::DomainDelete {
                key: DomainKey::new(&domains[1]),
            },
        ] {
            let view = render(screen, &page(&domains));

            assert!(!view.text.is_empty(), "{screen:?}");
            assert!(view.keyboard.rows.iter().all(|row| row.len() == 1), "{screen:?}");
        }
    }

    #[test]
    fn link_toggle_shows_the_state_and_sets_the_opposite() {
        let view = render(Screen::Settings, &page(&[]));

        assert_eq!(view.keyboard.rows[0][0].text, "🔗 Link in captions [✅]");
        assert_eq!(callbacks(&view)[0], "set_link_visibility:0");
    }

    #[test]
    fn current_language_is_ticked() {
        let view = render(Screen::Language, &page(&[]));
        let labels: Vec<&str> = view.keyboard.rows.iter().flatten().map(|button| button.text.as_str()).collect();

        assert_eq!(labels[..3], ["English [✅]", "Русский", "Українська"]);
        assert_eq!(callbacks(&view)[..3], ["set_language:en", "set_language:ru", "set_language:uk"]);
    }

    #[test]
    fn domain_list_offers_add_only_below_the_limit() {
        let below = domains(MAX_EXCLUDE_DOMAINS - 1);
        let full = domains(MAX_EXCLUDE_DOMAINS);

        assert!(callbacks(&render(Screen::Domains, &page(&below))).contains(&"open:domain_add"));
        assert!(!callbacks(&render(Screen::Domains, &page(&full))).contains(&"open:domain_add"));
    }

    #[test]
    fn domain_buttons_address_the_domain_by_fingerprint() {
        let domains = domains(2);
        let view = render(Screen::Domains, &page(&domains));

        assert_eq!(view.keyboard.rows[1][0].text, "site1.example");
        assert_eq!(callbacks(&view)[1], "open:domain.32ab1009d7fbd54d");
    }

    #[test]
    fn delete_is_confirmed_before_it_happens() {
        let domains = domains(1);
        let key = DomainKey::new(&domains[0]);
        let open_confirm = callbacks(&render(Screen::Domain { key }, &page(&domains)))[0].to_owned();
        let delete = callbacks(&render(Screen::DomainDelete { key }, &page(&domains)))[0].to_owned();

        assert_eq!(open_confirm, "open:domain_delete.811f612f78b710f2");
        assert_eq!(delete, "delete_domain:811f612f78b710f2");
        assert!(open_confirm.len() <= 64);
    }

    #[test]
    fn a_domain_screen_whose_domain_left_the_list_falls_back_to_the_list() {
        let domains = domains(2);
        let gone = DomainKey::new("gone.example");
        let kept = DomainKey::new(&domains[1]);

        assert_eq!(existing_screen(Screen::Domain { key: gone }, &domains), Screen::Domains);
        assert_eq!(existing_screen(Screen::DomainDelete { key: gone }, &domains), Screen::Domains);
        assert_eq!(
            existing_screen(Screen::Domain { key: kept }, &domains),
            Screen::Domain { key: kept }
        );
    }

    #[test]
    fn error_details_follow_the_notice_in_an_expandable_quote() {
        let notice = Notice {
            text: "Failed <add>".to_owned(),
            details: Some("Connection <refused>".to_owned()),
        };

        assert_eq!(
            with_notice(Some(notice), "Menu".to_owned()),
            "Failed &lt;add&gt;\n<blockquote expandable>Connection &lt;refused&gt;</blockquote>\n\nMenu"
        );
        assert_eq!(
            with_notice(Some(Notice::new("Added".to_owned())), "Menu".to_owned()),
            "Added\n\nMenu"
        );
        assert_eq!(with_notice(None, "Menu".to_owned()), "Menu");
    }

    #[tokio::test]
    async fn only_the_add_screen_waits_for_a_domain() {
        let fsm = Fsm::new(MemoryStorage::new(), telers::fsm::StorageKey::new(1, 2, 2, None, None));

        track_input(Screen::DomainAdd, &fsm).await;
        track_input(Screen::DomainAdd, &fsm).await;
        assert_eq!(fsm.get_states().await.unwrap().as_ref(), [MenuState::DomainInput.as_ref().into()]);

        track_input(Screen::Domains, &fsm).await;
        assert_eq!(fsm.get_state().await.unwrap(), None);
    }
}
