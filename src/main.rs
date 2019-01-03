extern crate reqwest;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate term_size;
extern crate termios;

use std::collections::HashMap;
use std::boxed::Box;
use std::error::Error;
use std::iter::Iterator;
use std::result::Result;
use std::os::unix::io::AsRawFd;
use std::io::Read;
use std::io::Write;

use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use termios::Termios;

#[derive(Debug, Deserialize)]
struct RedditAccessToken {
    access_token: String,
}

impl RedditAccessToken {
    fn get_client() -> RequestBuilder {
        Client::new()
            .post("https://www.reddit.com/api/v1/access_token")
            .header("User-Agent", "rust_reddit_browser-0.1.0")
    }

    fn get_request() -> RequestBuilder {
        let mut oauth_request = HashMap::new();
        let reddit_username = std::env::var("REDDIT_USERNAME").unwrap();
        let reddit_password = std::env::var("REDDIT_PASSWORD").unwrap();
        oauth_request.insert("grant_type", "password");
        oauth_request.insert("username", &reddit_username);
        oauth_request.insert("password", &reddit_password);

        Self::get_client()
            .basic_auth("eB_eu6_0qMDXxg", Some("rhrBt4faY0cuwU3CIHKi3Occ9rk"))
            .form(&oauth_request)
    }

    fn get_access_token() -> Result<RedditAccessToken, Box<Error>> {
        Ok(Self::get_request().send()?.json()?)
    }
}

#[derive(Debug)]
struct RedditClient {
    access_token: RedditAccessToken,
}

#[derive(Debug, Deserialize)]
struct RedditPost {
    title: String,
    subreddit: String,
    score: i64,
    permalink: String,
}

#[derive(Debug)]
struct RedditPosts {
    posts: Vec<RedditPost>,
    next_posts: Option<String>,
}

struct RedditPostsIterator {
    client: RedditClient,
    subreddit: String,
    exhausted: bool,
    next_posts: Option<String>,
}

impl RedditPostsIterator {
    fn new(client: RedditClient, subreddit: String) -> Self {
        Self {
            client,
            subreddit,
            exhausted: false,
            next_posts: None,
        }
    }
}

impl Iterator for RedditPostsIterator {
    type Item = RedditPosts;

    fn next(self: &mut Self) -> Option<Self::Item> {
        if self.exhausted {
            return None;
        };

        let current_posts = self.client.api_info(&self.subreddit, &self.next_posts).ok();
        self.next_posts = match &current_posts {
            Some(posts) => posts.next_posts.clone(),
            None => {
                self.exhausted = true;
                None
            }
        };
        current_posts
    }
}

impl RedditClient {
    fn new(access_token: RedditAccessToken) -> RedditClient {
        RedditClient {
            access_token,
        }
    }

    fn reddit_get(self: &RedditClient, endpoint: &str) -> RequestBuilder {
        let client = Client::new();

        client
            .get(format!("https://oauth.reddit.com{}", endpoint).as_str())
            .header("User-Agent", "rust_reddit_browser-0.1.0")
            .header(
                "Authorization",
                format!("bearer {}", self.access_token.access_token),
            )
    }

    fn api_info(
        self: &RedditClient,
        subreddit: &str,
        after_option: &Option<String>,
    ) -> Result<RedditPosts, Box<Error>> {
        let mut request = self.reddit_get(format!("/r/{}/new.json", subreddit).as_str());

        if let Some(after) = after_option {
            request = request.query(&[("after", after)]);
        }

        let json = request.send()?.json::<Value>()?;

        let next: Option<String> = json["data"]["after"].as_str().map(|x| x.to_string());

        let result = || -> Option<Vec<RedditPost>> {
            Some(
                json.pointer("/data/children")?
                    .as_array()?
                    .iter()
                    .filter_map(|x| serde_json::from_value::<RedditPost>(x["data"].clone()).ok())
                    .collect::<Vec<RedditPost>>(),
            )
        }();

        Ok(RedditPosts {
            posts: result.unwrap(),
            next_posts: next,
        })
    }
}

struct TerminalRenderer {
    iterator: Box<Iterator<Item=RedditPost>>,
    buffer: Vec<RedditPost>
}

impl TerminalRenderer {
    fn new(iterator : impl Iterator<Item=RedditPost> + 'static) -> TerminalRenderer {
        TerminalRenderer { iterator: Box::new(iterator), buffer: Vec::new() }
    }

    fn fill_buffer(self: &mut Self, amount: usize) {
        while self.buffer.len() < amount {
            self.buffer.push(self.iterator.next().unwrap())
        }
    }

    fn render(self: &mut Self, screen: &Screen) {
        self.fill_buffer((screen.upper_line + screen.height) as usize);

        println!("[2J");

        let mut i = 0;
        for post in &mut self.buffer {
            if i < screen.upper_line {
                i += 1;
                continue;
            }

            if i == screen.line {
                let mut output = format!("{2:6} | {1:10} | {0}", post.title, post.subreddit, post.score);
                output.truncate(screen.width as usize);
                print!(
                    "[7m{}\nhttps://reddit.com{}[0m", output, post.permalink
                );
            } else {
                let mut output = format!("{2:6} | {1:10} | {0}", post.title, post.subreddit, post.score);
                output.truncate(screen.width as usize);
                print!(
                    "{}", output
                );
            }
            i += 1;
            if i == screen.height + screen.upper_line - 1 {
                break;
            }
            println!();
        }
        std::io::stdout().flush().unwrap();
    }
}

#[derive(Clone,Debug)]
struct Screen {
    line: i32,
    upper_line: i32,
    height: i32,
    width: i32
}

impl Screen {
    fn new() -> Self {
        let (w, h) = term_size::dimensions().unwrap();

        Self {
            line: 0,
            upper_line: 0,
            height: h as i32,
            width: w as i32
        }
    }

    fn down(self: Self) -> Self {
        let mut clone = self.clone();
        clone.line += 1;
        if clone.line + 2 >= clone.height + clone.upper_line {
            clone.upper_line += 1;
        }
        clone
    }

    fn up(self: Self) -> Self {
        let mut clone = self.clone();
        clone.line = std::cmp::max(0, clone.line - 1);
        if clone.line - 1 < clone.upper_line {
            clone.upper_line = std::cmp::max(0, clone.upper_line - 1);
        }
        clone
    }
}

fn main() {
    let reddit_access_token = RedditAccessToken::get_access_token().unwrap();
    let reddit_client = RedditClient::new(reddit_access_token);
    let iterator = RedditPostsIterator::new(reddit_client, std::env::args().nth(1).unwrap());
    let mut renderer = TerminalRenderer::new(iterator.flat_map(|x| x.posts));
    let mut screen = Screen::new();
    let stdin_fd = std::io::stdin().as_raw_fd();
    let mut termios = Termios::from_fd(stdin_fd).unwrap();

    termios.c_lflag &= !(termios::ICANON | termios::ECHO);
    termios::tcsetattr(stdin_fd, termios::TCSANOW, &termios).unwrap();

    loop {
        renderer.render(&screen);
        match std::io::stdin().bytes().next().unwrap().unwrap() as char {
            'j' => screen = screen.down(),
            'k' => screen = screen.up(),
            'q' => return,
            _ => (),
        }
    }
}
