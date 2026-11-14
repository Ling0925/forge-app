use std::{
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

pub type Sleep = Pin<Box<dyn Future<Output = ()> + Send>>;

pub trait Clock: Send + Sync {
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration) -> Sleep;
}

#[derive(Default)]
pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Duration {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock is after Unix epoch")
    }
    fn sleep(&self, duration: Duration) -> Sleep {
        Box::pin(tokio::time::sleep(duration))
    }
}

#[derive(Clone)]
pub struct FakeClock {
    time: watch::Sender<Duration>,
}

impl Default for FakeClock {
    fn default() -> Self {
        let (time, _) = watch::channel(Duration::ZERO);
        Self { time }
    }
}

impl FakeClock {
    pub fn advance(&self, duration: Duration) {
        self.time.send_modify(|time| *time += duration);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Duration {
        *self.time.borrow()
    }

    fn sleep(&self, duration: Duration) -> Sleep {
        let mut time = self.time.subscribe();
        let deadline = *time.borrow() + duration;
        let clock = self.clone();
        Box::pin(async move {
            while *time.borrow_and_update() < deadline {
                time.changed().await.expect("sleep retains its clock");
            }
            drop(clock);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::task::{Context, Poll, Waker};

    fn pending(sleep: &mut Sleep) -> bool {
        matches!(
            sleep.as_mut().poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        )
    }

    #[tokio::test]
    async fn fake_sleep_waits_until_deadline() {
        let clock = FakeClock::default();
        let mut sleep = clock.sleep(Duration::from_secs(2));
        assert!(pending(&mut sleep));
        clock.advance(Duration::from_secs(1));
        assert!(pending(&mut sleep));
        clock.advance(Duration::from_secs(1));
        sleep.await;
        assert_eq!(clock.now(), Duration::from_secs(2));
    }

    #[tokio::test]
    async fn independent_sleepers_wake_and_pre_poll_advancement_is_observed() {
        let clock = FakeClock::default();
        let early = clock.sleep(Duration::from_secs(1));
        let mut late = clock.sleep(Duration::from_secs(2));
        clock.clone().advance(Duration::from_secs(1));
        early.await;
        assert!(pending(&mut late));
        clock.advance(Duration::from_secs(1));
        late.await;
        clock.sleep(Duration::ZERO).await;
    }

    #[tokio::test]
    async fn advancing_wakes_a_spawned_sleeper() {
        let clock = FakeClock::default();
        let sleep = clock.sleep(Duration::from_secs(2));
        let task = tokio::spawn(sleep);
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        clock.advance(Duration::from_secs(2));
        task.await.unwrap();
    }

    #[tokio::test]
    async fn real_clock_sleeps_and_moves_forward() {
        let clock = RealClock;
        let start = clock.now();
        let duration = Duration::from_millis(1);
        clock.sleep(duration).await;
        assert!(clock.now() - start >= duration);
    }
}
