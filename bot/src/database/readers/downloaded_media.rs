use async_trait::async_trait;
use sea_orm::{
    prelude::Expr, ColumnTrait as _, ConnectionTrait, EntityTrait as _, ExprTrait as _, FromQueryResult, QueryFilter as _, QueryOrder as _,
    QuerySelect as _,
};
use std::convert::Infallible;
use time::{Duration, OffsetDateTime};

use crate::{
    database::{
        interfaces::downloaded_media::DownloadedMediaReader,
        models::{downloaded_media, sea_orm_active_enums},
    },
    entities::{DownloadedMedia, DownloadedMediaByDomainCount, DownloadedMediaCount, DownloadedMediaStats},
    errors::ErrorKind,
    value_objects::MediaType,
};

const DOMAIN_MATCHES: &str = "(lower(domain) = lower($1) OR right(lower($1), length(domain) + 1) = ('.' || lower(domain)))";

// ids are free text (generic-extractor ids carry file names), so escape them before use as a regex
fn url_contains_token(column: &str) -> String {
    format!(
        r"({column} <> '' AND strpos($1, {column}) > 0 AND $1 ~ ('(^|[/?&=])' || regexp_replace({column}, '([\[\]\\.^$*+?(){{}}|])', '\\\1', 'g') || '([&?/#]|$)'))"
    )
}

pub struct SeaOrmDownloadedMediaReader<'a, Conn> {
    conn: &'a Conn,
}

impl<'a, Conn> SeaOrmDownloadedMediaReader<'a, Conn> {
    pub const fn new(conn: &'a Conn) -> Self {
        Self { conn }
    }
}

#[async_trait]
impl<Conn: ConnectionTrait> DownloadedMediaReader for SeaOrmDownloadedMediaReader<'_, Conn> {
    async fn get(
        &self,
        search: &str,
        domain: Option<&str>,
        audio_language: Option<&str>,
        media_type: MediaType,
        crop_start_time: Option<i32>,
        crop_end_time: Option<i32>,
    ) -> Result<Option<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{AudioLanguage, CropEndTime, CropStartTime, DisplayId, Domain, Id, MediaType},
            Entity,
        };

        let mut query = Entity::find()
            .filter(MediaType.eq(sea_orm_active_enums::MediaType::from(media_type)))
            .filter(
                // if `search` is ID
                Expr::col(Id)
                    .eq(search)
                    .or(Expr::col(DisplayId).eq(search))
                    // if `search` is URL
                    .or(Expr::cust_with_values(url_contains_token("id"), [search]))
                    .or(Expr::cust_with_values(url_contains_token("display_id"), [search])),
            );
        if let Some(lang) = audio_language {
            query = query.filter(AudioLanguage.eq(lang));
        }
        query = match domain {
            Some(domain) => query.filter(Expr::cust_with_values(DOMAIN_MATCHES, [domain])),
            None => query.filter(Domain.is_null()),
        };
        if let Some(time) = crop_start_time {
            query = query.filter(CropStartTime.eq(time));
        } else {
            query = query.filter(CropStartTime.is_null());
        }
        if let Some(time) = crop_end_time {
            query = query.filter(CropEndTime.eq(time));
        } else {
            query = query.filter(CropEndTime.is_null());
        }

        Ok(query.one(self.conn).await?.map(Into::into))
    }

    async fn get_random(
        &self,
        limit: u64,
        media_type: MediaType,
        domains: &[String],
    ) -> Result<Vec<DownloadedMedia>, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{Domain, MediaType},
            Entity,
        };

        Ok(Entity::find()
            .filter(MediaType.eq(sea_orm_active_enums::MediaType::from(media_type)))
            .filter(Domain.is_in(domains))
            .order_by_desc(Expr::cust("RANDOM()"))
            .limit(limit)
            .all(self.conn)
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }

    async fn get_stats(&self, top_domains_limit: u64) -> Result<DownloadedMediaStats, ErrorKind<Infallible>> {
        use downloaded_media::{
            Column::{CreatedAt, Domain, FileId},
            Entity,
        };

        #[derive(Default, Debug, FromQueryResult)]
        struct CountResult {
            count: i64,
        }

        #[derive(Debug, FromQueryResult)]
        struct DomainCountResult {
            domain: String,
            count: i64,
        }

        async fn count_by_period<Conn>(conn: &Conn, since: Option<OffsetDateTime>) -> Result<DownloadedMediaCount, ErrorKind<Infallible>>
        where
            Conn: ConnectionTrait,
        {
            let mut query = Entity::find().select_only().expr_as(Expr::col(FileId).count(), "count");
            if let Some(since) = since {
                query = query.filter(Expr::col(CreatedAt).gte(since));
            }
            let count = query.into_model::<CountResult>().one(conn).await?.unwrap_or_default().count;
            Ok(DownloadedMediaCount { count })
        }

        let now = OffsetDateTime::now_utc();

        let count_total = count_by_period(self.conn, None).await?;
        let count_last_day = count_by_period(self.conn, Some(now - Duration::days(1))).await?;
        let count_last_week = count_by_period(self.conn, Some(now - Duration::days(7))).await?;
        let count_last_month = count_by_period(self.conn, Some(now - Duration::days(30))).await?;
        let top_domains = Entity::find()
            .select_only()
            .column(Domain)
            .expr_as(Expr::col(Domain).count(), "count")
            .filter(Expr::col(Domain).is_not_null())
            .group_by(Domain)
            .order_by_desc(Expr::col("count"))
            .limit(Some(top_domains_limit))
            .into_model::<DomainCountResult>()
            .all(self.conn)
            .await?
            .into_iter()
            .map(|val| DownloadedMediaByDomainCount {
                domain: val.domain,
                count: val.count,
            })
            .collect();

        Ok(DownloadedMediaStats {
            last_day: count_last_day,
            last_week: count_last_week,
            last_month: count_last_month,
            total: count_total,
            top_domains,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::SeaOrmDownloadedMediaReader;
    use crate::{
        database::{
            interfaces::downloaded_media::{DownloadedMediaReader as _, DownloadedMediaRepo as _},
            repos::downloaded_media::SeaOrmDownloadedMediaRepo,
        },
        entities::DownloadedMedia,
        value_objects::MediaType,
    };

    use migration::{Migrator, MigratorTrait as _};
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use testcontainers_modules::{
        postgres::Postgres,
        testcontainers::{runners::AsyncRunner as _, ContainerAsync, ImageExt as _},
    };
    use time::OffsetDateTime;

    // Generic-extractor style id (file name plus query); `[ABC-123]` is an invalid regex range
    const FREE_TEXT_ID: &str = "12345678-720p.mp4?dload=SITE.COM - [aBcDeFgHiJk] [ABC-123] Some Title (720)";
    const CDN_DOMAIN: &str = "cdn-node.example.com";
    const YT_ID: &str = "abcdefghijk";
    const YT_DOMAIN: Option<&str> = Some("youtube.com");
    const EXAMPLE: Option<&str> = Some("example.com");

    struct Db {
        _container: ContainerAsync<Postgres>,
        conn: DatabaseConnection,
    }

    impl Db {
        async fn with_rows(rows: Vec<DownloadedMedia>) -> Self {
            let container = Postgres::default().with_tag("18-alpine").start().await.unwrap();
            let port = container.get_host_port_ipv4(5432).await.unwrap();
            let conn = Database::connect(ConnectOptions::new(format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres")))
                .await
                .unwrap();
            Migrator::up(&conn, None).await.unwrap();
            let repo = SeaOrmDownloadedMediaRepo::new(&conn);
            for row in rows {
                repo.insert_or_ignore(row).await.unwrap();
            }
            Self {
                _container: container,
                conn,
            }
        }

        async fn find(&self, search: &str, domain: Option<&str>) -> Option<String> {
            self.find_with(search, domain, MediaType::Video, None, None, None).await
        }

        async fn find_with(
            &self,
            search: &str,
            domain: Option<&str>,
            media_type: MediaType,
            audio_language: Option<&str>,
            crop_start_time: Option<i32>,
            crop_end_time: Option<i32>,
        ) -> Option<String> {
            SeaOrmDownloadedMediaReader::new(&self.conn)
                .get(search, domain, audio_language, media_type, crop_start_time, crop_end_time)
                .await
                .unwrap()
                .map(|media| media.file_id)
        }

        async fn assert_cases(&self, domain: Option<&str>, cases: &[(&str, Option<&str>)]) {
            for (url, expected) in cases {
                assert_eq!(self.find(url, domain).await.as_deref(), *expected, "{url}");
            }
        }
    }

    fn video(file_id: &str, id: &str, display_id: Option<&str>, domain: Option<&str>) -> DownloadedMedia {
        DownloadedMedia {
            file_id: file_id.to_owned(),
            id: id.to_owned(),
            display_id: display_id.map(ToOwned::to_owned),
            domain: domain.map(ToOwned::to_owned),
            media_type: MediaType::Video,
            created_at: OffsetDateTime::now_utc(),
            audio_language: None,
            crop_start_time: None,
            crop_end_time: None,
        }
    }

    fn long_free_text_id() -> String {
        format!("{}tail", "[Part-9] (x)*.+?|{2}\\ ".repeat(60))
    }

    #[tokio::test]
    async fn ip_and_unrelated_urls_match_nothing() {
        let db = Db::with_rows(vec![
            video("any", ".*", None, Some("wild.example")),
            video("some", ".+", None, Some("wild.example")),
            video("alt", "|", None, Some("wild.example")),
            video("empty-id", "", None, Some("wild.example")),
            video("empty-display", "someid", Some(""), Some("wild.example")),
            video("free-text", FREE_TEXT_ID, Some(FREE_TEXT_ID), Some(CDN_DOMAIN)),
            video("long", &long_free_text_id(), None, Some(CDN_DOMAIN)),
            video("nodomain", "abc", None, None),
            video("example", "zzzzzzzzzzz", None, EXAMPLE),
        ])
        .await;

        let urls = [
            "https://127.0.0.1/",
            "https://0.0.0.10/",
            "https://[::1]/",
            "https://x/",
            "https://example.com/",
            "https://example.com/watch?v=yyyyyyyyyyy",
            "https://wild.example/anything",
        ];
        for url in urls {
            for domain in [None, Some("127.0.0.1"), Some("wild.example"), EXAMPLE, Some(CDN_DOMAIN)] {
                assert_eq!(db.find(url, domain).await, None, "{url} {domain:?}");
            }
        }
    }

    #[tokio::test]
    async fn invalid_regex_id_neither_breaks_nor_leaks() {
        let db = Db::with_rows(vec![video("free-text", FREE_TEXT_ID, Some(FREE_TEXT_ID), Some(CDN_DOMAIN))]).await;
        let domain = Some(CDN_DOMAIN);

        assert_eq!(db.find(&format!("https://{CDN_DOMAIN}/other.mp4"), domain).await, None);
        assert_eq!(
            db.find(&format!("https://{CDN_DOMAIN}/{FREE_TEXT_ID}"), domain).await.as_deref(),
            Some("free-text")
        );
        assert_eq!(db.find(FREE_TEXT_ID, domain).await.as_deref(), Some("free-text"));
    }

    #[tokio::test]
    async fn long_free_text_id_is_matched_literally() {
        let id = long_free_text_id();
        let db = Db::with_rows(vec![video("long", &id, None, Some(CDN_DOMAIN))]).await;
        let domain = Some(CDN_DOMAIN);

        assert_eq!(db.find(&format!("https://{CDN_DOMAIN}/[Part-9] (x)"), domain).await, None);
        assert_eq!(db.find(&format!("https://{CDN_DOMAIN}/{id}"), domain).await.as_deref(), Some("long"));
        assert_eq!(db.find(&id, domain).await.as_deref(), Some("long"));
    }

    #[tokio::test]
    async fn metacharacters_in_ids_are_literal() {
        let db = Db::with_rows(vec![
            video("plus", "a+b", None, EXAMPLE),
            video("pipe", "x|y", None, EXAMPLE),
            video("parens", "clip (720)", None, EXAMPLE),
            video("dot", "dot.id", None, EXAMPLE),
            video("star", "a.b*", None, EXAMPLE),
            video("brace", "ab{2}", None, EXAMPLE),
            video("class", "[abc]", None, EXAMPLE),
            video("caret", "^top", None, EXAMPLE),
            video("dollar", "end$", None, EXAMPLE),
            video("escape", r"\d+", None, EXAMPLE),
            video("backslash", r"back\slash", None, EXAMPLE),
            video("question", "q?", None, EXAMPLE),
            video("cyrillic", "видео", None, EXAMPLE),
        ])
        .await;

        db.assert_cases(
            EXAMPLE,
            &[
                ("https://example.com/a+b", Some("plus")),
                ("https://example.com/ab", None),
                ("https://example.com/aab", None),
                ("https://example.com/x|y", Some("pipe")),
                ("https://example.com/x", None),
                ("https://example.com/y", None),
                ("https://example.com/clip (720)", Some("parens")),
                ("https://example.com/clip 720", None),
                ("https://example.com/dot.id", Some("dot")),
                ("https://example.com/dotXid", None),
                ("https://example.com/a.b*", Some("star")),
                ("https://example.com/a.bbb", None),
                ("https://example.com/axb", None),
                ("https://example.com/ab{2}", Some("brace")),
                ("https://example.com/abb", None),
                ("https://example.com/[abc]", Some("class")),
                ("https://example.com/a", None),
                ("https://example.com/^top", Some("caret")),
                ("https://example.com/top", None),
                ("https://example.com/end$", Some("dollar")),
                ("https://example.com/end", None),
                (r"https://example.com/\d+", Some("escape")),
                ("https://example.com/123", None),
                (r"https://example.com/back\slash", Some("backslash")),
                ("https://example.com/backslash", None),
                ("https://example.com/q?", Some("question")),
                ("https://example.com/q", None),
                ("https://example.com/видео", Some("cyrillic")),
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn id_must_be_a_whole_token() {
        let db = Db::with_rows(vec![video("row", "abc", None, EXAMPLE)]).await;

        db.assert_cases(
            EXAMPLE,
            &[
                ("abc", Some("row")),
                ("https://example.com/abc", Some("row")),
                ("https://example.com/abc/", Some("row")),
                ("https://example.com/v/abc?x=1", Some("row")),
                ("https://example.com/?v=abc", Some("row")),
                ("https://example.com/?a=1&v=abc", Some("row")),
                ("https://example.com/?v=abc&a=1", Some("row")),
                ("https://example.com/v/abc#t=10", Some("row")),
                ("https://example.com/abcd", None),
                ("https://example.com/xabc", None),
                ("https://example.com/?v=abc1", None),
                ("https://example.com/?v=1abc", None),
                ("https://example.com/?vabc=1", None),
                ("https://example.com/?x=abc-1", None),
                ("https://example.com/ABC", None),
                ("ab", None),
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn common_url_shapes_find_the_id() {
        let db = Db::with_rows(vec![video("yt", YT_ID, None, YT_DOMAIN)]).await;

        db.assert_cases(
            Some("www.youtube.com"),
            &[
                ("https://www.youtube.com/watch?v=abcdefghijk", Some("yt")),
                ("https://www.youtube.com/shorts/abcdefghijk", Some("yt")),
                ("https://www.youtube.com/embed/abcdefghijk?feature=share", Some("yt")),
                ("https://www.youtube.com/watch?v=abcdefghijk&list=PL123&index=2", Some("yt")),
                ("https://www.youtube.com/watch?list=PL123&v=abcdefghijk", Some("yt")),
                ("https://www.youtube.com/watch?v=abcdefghijk#t=30", Some("yt")),
                ("https://www.youtube.com/watch?v=ABCDEFGHIJK", None),
                ("https://www.youtube.com/watch?v=abcdefghijkl", None),
                ("https://www.youtube.com/channel/UCabcdefghijk", None),
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn display_id_is_searched_too() {
        let db = Db::with_rows(vec![
            video("num", "9876543", Some("my-video-slug"), EXAMPLE),
            video("nullslug", "1122334455", None, EXAMPLE),
        ])
        .await;

        db.assert_cases(
            EXAMPLE,
            &[
                ("https://example.com/video/my-video-slug", Some("num")),
                ("https://example.com/v/9876543", Some("num")),
                ("my-video-slug", Some("num")),
                ("https://example.com/video/my-video", None),
                ("https://example.com/video/my-video-slug-2", None),
                ("https://example.com/p/1122334455", Some("nullslug")),
                ("https://example.com/p/112233445", None),
            ],
        )
        .await;
    }

    #[tokio::test]
    async fn exact_id_search_respects_media_type() {
        let mut audio = video("audio", YT_ID, None, YT_DOMAIN);
        audio.media_type = MediaType::Audio;
        let db = Db::with_rows(vec![video("yt", YT_ID, Some("display-slug"), YT_DOMAIN), audio]).await;

        assert_eq!(db.find(YT_ID, YT_DOMAIN).await.as_deref(), Some("yt"));
        assert_eq!(db.find("display-slug", YT_DOMAIN).await.as_deref(), Some("yt"));
        assert_eq!(db.find("abcdefghij", YT_DOMAIN).await, None);
        assert_eq!(
            db.find_with(YT_ID, YT_DOMAIN, MediaType::Audio, None, None, None).await.as_deref(),
            Some("audio")
        );
        assert_eq!(db.find_with(YT_ID, YT_DOMAIN, MediaType::Photo, None, None, None).await, None);
    }

    #[tokio::test]
    async fn domain_must_equal_or_be_a_parent_of_the_host() {
        let db = Db::with_rows(vec![
            video("yt", YT_ID, None, YT_DOMAIN),
            video("nodomain", YT_ID, None, None),
            video("legacy", "legacyid123", None, Some("www.legacy.example")),
        ])
        .await;
        let url = "https://host/watch?v=abcdefghijk";

        for host in [
            "youtube.com",
            "www.youtube.com",
            "m.youtube.com",
            "a.b.youtube.com",
            "YouTube.com",
            "WWW.YOUTUBE.COM",
        ] {
            assert_eq!(db.find(url, Some(host)).await.as_deref(), Some("yt"), "{host}");
        }
        for host in ["notyoutube.com", "youtubeXcom", "youtube.com.evil", "youtube.co", "youtu.be", "127.0.0.1"] {
            assert_eq!(db.find(url, Some(host)).await, None, "{host}");
        }
        assert_eq!(db.find(url, None).await.as_deref(), Some("nodomain"));

        let legacy_url = "https://host/v/legacyid123";
        assert_eq!(db.find(legacy_url, Some("www.legacy.example")).await.as_deref(), Some("legacy"));
        assert_eq!(db.find(legacy_url, Some("legacy.example")).await, None);
        assert_eq!(db.find(legacy_url, None).await, None);
    }

    #[tokio::test]
    async fn language_and_sections_narrow_the_match() {
        let mut cropped = video("cropped", "aaaaaaaaaaa", None, YT_DOMAIN);
        cropped.crop_start_time = Some(1);
        cropped.crop_end_time = Some(2);
        let mut ru = video("ru", "bbbbbbbbbbb", None, YT_DOMAIN);
        ru.audio_language = Some("ru".to_owned());
        let db = Db::with_rows(vec![video("plain", "aaaaaaaaaaa", None, YT_DOMAIN), cropped, ru]).await;
        let url_a = "https://youtube.com/watch?v=aaaaaaaaaaa";
        let url_b = "https://youtube.com/watch?v=bbbbbbbbbbb";
        let find = |url, lang, start, end| db.find_with(url, YT_DOMAIN, MediaType::Video, lang, start, end);

        assert_eq!(find(url_a, None, None, None).await.as_deref(), Some("plain"));
        assert_eq!(find(url_a, None, Some(1), Some(2)).await.as_deref(), Some("cropped"));
        assert_eq!(find(url_a, None, Some(1), None).await, None);
        assert_eq!(find(url_a, None, Some(2), Some(3)).await, None);
        assert_eq!(find(url_a, Some("ru"), None, None).await, None);
        assert_eq!(find(url_b, None, None, None).await.as_deref(), Some("ru"));
        assert_eq!(find(url_b, Some("ru"), None, None).await.as_deref(), Some("ru"));
        assert_eq!(find(url_b, Some("en"), None, None).await, None);
        assert_eq!(find(url_b, Some("ru"), Some(1), Some(2)).await, None);
    }
}
