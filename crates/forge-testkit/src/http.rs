use std::{collections::BTreeMap, io};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::{mpsc, oneshot},
    task::JoinHandle,
};

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub chunks: Vec<Vec<u8>>,
    pub finish: bool,
}
impl MockResponse {
    pub fn sse(body: &str) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            chunks: vec![body.as_bytes().to_vec()],
            finish: true,
        }
    }
}
pub struct MockServer {
    pub endpoint: String,
    /// First request only, for single-request call sites.
    pub request: oneshot::Receiver<RecordedRequest>,
    /// Every request, in order.
    pub requests: mpsc::UnboundedReceiver<RecordedRequest>,
    pub task: JoinHandle<io::Result<()>>,
}
impl MockServer {
    /// Serve a single response to a single request.
    pub async fn start(response: MockResponse) -> Self {
        Self::start_all(vec![response]).await
    }
    /// Serve each response to one request, in order. The first request is
    /// recorded on `request`; every request is sent to `requests`.
    pub async fn start_all(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (sender, request) = oneshot::channel();
        let (recorded, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let mut sender = Some(sender);
            for response in responses {
                let (socket, _) = listener.accept().await?;
                let mut socket = BufReader::new(socket);
                let mut line = String::new();
                socket.read_line(&mut line).await?;
                let mut parts = line.split_whitespace();
                let method = parts.next().unwrap().to_owned();
                let path = parts.next().unwrap().to_owned();
                let mut headers = BTreeMap::new();
                loop {
                    line.clear();
                    socket.read_line(&mut line).await?;
                    if line == "\r\n" {
                        break;
                    }
                    let (name, value) = line.trim_end().split_once(':').unwrap();
                    headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
                }
                let length = headers.get("content-length").unwrap().parse().unwrap();
                let mut body = vec![0; length];
                socket.read_exact(&mut body).await?;
                let recorded_request = RecordedRequest {
                    method,
                    path,
                    headers,
                    body,
                };
                if let Some(sender) = sender.take() {
                    sender.send(recorded_request.clone()).unwrap();
                }
                recorded.send(recorded_request).unwrap();
                socket.write_all(format!("HTTP/1.1 {} Response\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n",response.status).as_bytes()).await?;
                for (name, value) in response.headers {
                    socket
                        .write_all(format!("{name}: {value}\r\n").as_bytes())
                        .await?;
                }
                socket.write_all(b"\r\n").await?;
                for chunk in response.chunks {
                    socket
                        .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                        .await?;
                    socket.write_all(&chunk).await?;
                    socket.write_all(b"\r\n").await?;
                    socket.flush().await?;
                    tokio::task::yield_now().await;
                }
                if response.finish {
                    socket.write_all(b"0\r\n\r\n").await?;
                }
                socket.shutdown().await?;
            }
            Ok(())
        });
        Self {
            endpoint,
            request,
            requests,
            task,
        }
    }
}
