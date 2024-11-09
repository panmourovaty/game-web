#![forbid(unsafe_code)]

use mimalloc::MiMalloc;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use askama::Template;
use axum::{
    extract::Query, response::Html, routing::{get, post}, Extension, Form, Router
};
use memory_serve::{load_assets, MemoryServe};
use minify_html::{minify, Cfg};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use sqlx::Row;
use dotenv::dotenv;

#[tokio::main]
async fn main() {
    dotenv().ok();
    let db_connection_string = std::env::var("DATABASE_CONNECTION_STRING").expect("DATABASE_CONNECTION_STRING not set!");

    let pool = PgPoolOptions::new()
    .max_connections(400)
    .connect(&db_connection_string)
    .await
    .unwrap();

    let asset_router = MemoryServe::new(load_assets!("templates/assets"))
        .cache_control(memory_serve::CacheControl::Medium)
        .into_router();
    let app = Router::new()
        .route("/", get(games_page))
        .route("/game_window", get(games))
        .route("/admin/dashboard", get(admin_dashboard))
        .route("/admin/save_game", post(update_db))
        .merge(asset_router)
        .layer(Extension(pool));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Listening on: 8080 | http://localhost:8080");
    axum::serve(listener, app).await.unwrap();
}

fn minifi_html(html: String) -> Vec<u8> {
    let mut cfg = Cfg::new();
    cfg.keep_comments = false;
    cfg.minify_css = true;
    cfg.minify_js = true;
    let result: Vec<u8> = minify(html.as_bytes(), &cfg);
    result
}

#[derive(Serialize, Deserialize)]
struct DBGameInfo {
    id: i32,
    name: String,
    description: String,
    year_released: i32,
    completion_order: i32,
    image_cover: String,
    dlc: bool,
    genres: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FormDBGameInfo {
    game_id: i32,
    game_name: String,
    game_description: Option<String>,
    year_released: Option<String>,
    completion_order: i32,
    image_cover: Option<String>,
    dlc: Option<String>,
    genres: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct GenresFromDB {
    id: i32,
    name: String
}

async fn load_info_from_db_filtered(Extension(pool): Extension<PgPool>, asc: bool, genre_name: String) -> Vec<DBGameInfo> {
    let order = if asc { "ASC" } else { "DESC" };

    let query = if genre_name.is_empty() {
        format!(
            r#"
            SELECT g.id, g.name, g.description, g.year_released, g.completion_order, g.image_cover, g.dlc, array_agg(ge.name) AS genres
            FROM public.games g
            LEFT JOIN public.game_genres gg ON g.id = gg.game_id
            LEFT JOIN public.genres ge ON gg.genre_id = ge.id
            GROUP BY g.id
            ORDER BY g.completion_order {};
            "#,
            order
        )
    } else {
        format!(
            r#"
            SELECT g.id, g.name, g.description, g.year_released, g.completion_order, g.image_cover, g.dlc, array_agg(ge.name) AS genres
            FROM public.games g
            LEFT JOIN public.game_genres gg ON g.id = gg.game_id
            LEFT JOIN public.genres ge ON gg.genre_id = ge.id
            WHERE g.id IN (
                SELECT g2.id
                FROM public.games g2
                LEFT JOIN public.game_genres gg2 ON g2.id = gg2.game_id
                LEFT JOIN public.genres ge2 ON gg2.genre_id = ge2.id
                WHERE ge2.name = $1
            )
            GROUP BY g.id
            ORDER BY g.completion_order {};
            "#,
            order
        )
    };

    let db_info = if genre_name.is_empty() {
        sqlx::query(&query)
            .fetch_all(&pool)
            .await
            .unwrap()
    } else {
        sqlx::query(&query)
            .bind(&genre_name) 
            .fetch_all(&pool)
            .await
            .unwrap()
    };

    db_info.into_iter().map(|record| DBGameInfo { 
        id: record.try_get::<i32, _>("id").unwrap(),
        name: record.try_get::<String, _>("name").unwrap(),
        description: record.try_get::<Option<String>, _>("description").unwrap_or(None).unwrap_or_default(),
        year_released: record.try_get::<Option<i32>, _>("year_released").unwrap_or(None).unwrap_or(-1),
        completion_order: record.try_get::<Option<i32>, _>("completion_order").unwrap_or(Some(-1)).unwrap(),
        image_cover: record.try_get::<Option<String>, _>("image_cover").unwrap_or(None).unwrap_or_default(),
        dlc: record.try_get::<Option<bool>, _>("dlc").unwrap_or(Some(false)).unwrap(),
        genres: record.try_get::<Option<Vec<String>>, _>("genres").unwrap_or(Some(Vec::new())).unwrap(),
    }).collect()
}

async fn load_genres_from_db(Extension(pool): Extension<PgPool>) -> Vec<GenresFromDB> {
    let db_info = sqlx::query!(
        r#"
        SELECT g.id, g.name
        FROM genres g
        ORDER BY g.name ASC;
        "#
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    db_info.into_iter().map(|record| GenresFromDB {
        id: record.id,
        name: record.name,
    }).collect()
}

#[derive(Serialize, Deserialize)]
struct GamesQueryParams{
    filter: Option<String>,
    asc: Option<bool>,
}

#[derive(Template)]
#[template(path = "pages/game_window.html", escape = "none")]
struct GamesTemplate {
    games: Vec<DBGameInfo>,
}
async fn games(Extension(pool): Extension<PgPool>, Query(params): Query<GamesQueryParams>) -> Html<Vec<u8>> {
    let filter = params.filter.unwrap_or_else(|| "".to_string());
    let asc = params.asc.unwrap_or(false);
    let gameinfos = load_info_from_db_filtered(Extension(pool), asc, filter).await;
    let template = GamesTemplate{games: gameinfos};
    Html(minifi_html(template.render().unwrap()))
}

#[derive(Template)]
#[template(path = "pages/games.html", escape = "none")]
struct GamesPageTemplate {
}
async fn games_page() -> Html<Vec<u8>> {
    let template = GamesPageTemplate{};
    Html(minifi_html(template.render().unwrap()))
}

#[derive(Template)]
#[template(path = "pages/admin/dashboard.html", escape = "none")]
struct AdminDashboardTemplate{
    games: Vec<DBGameInfo>,
    genres: Vec<GenresFromDB>
}
async fn admin_dashboard(Extension(pool): Extension<PgPool>) -> Html<Vec<u8>>{
    let gameinfos = load_info_from_db_filtered(Extension(pool.clone()), false, "".to_string()).await;
    let _genres = load_genres_from_db(Extension(pool.clone())).await;
    let template = AdminDashboardTemplate{games: gameinfos, genres: _genres};
    Html(minifi_html(template.render().unwrap()))
}

async fn update_db(Extension(pool): Extension<PgPool>, inputform: Form<FormDBGameInfo>) -> axum::response::Html<String>{
    
    println!("Received game info: {:?}", inputform);

    let query = format!(
        "UPDATE games SET name = {}, description = {}, year_released = {}, completion_order = {}, image_cover = {}, dlc = {} WHERE id = {};",
        format!("'{}'", inputform.game_name),
        match &inputform.game_description {
            Some(ref description) if !description.is_empty() => format!("'{}'", description),
            Some(_) => "NULL".to_string(),
            None => "NULL".to_string(),
        },
        match inputform.year_released {
            Some(ref year) => year.to_string(),
            None => "NULL".to_string(),
        },
        inputform.completion_order,
        match &inputform.image_cover {
            Some(ref image) if !image.is_empty() => format!("'{}'", image),
            Some(_) => "NULL".to_string(),
            None => "NULL".to_string(),
        },
        match inputform.dlc.as_deref(){
            Some("on") => true,
            _ => false,
        },
        inputform.game_id
    );

    println!("Received game query: {:?}", query);

    let _ = sqlx::query(&query)
    .execute(&pool)
    .await;

    Html("OK".to_string())
}