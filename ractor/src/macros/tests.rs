use crate::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

#[derive(Debug)]
enum TestMessage {
    Add(i32, i32, RpcReplyPort<i32>),
    Echo(String, RpcReplyPort<String>),
    #[allow(dead_code)]
    CastOnly(i32),
}

#[cfg(feature = "cluster")]
impl crate::Message for TestMessage {}

struct TestActor;

#[cfg_attr(feature = "async-trait", ractor::async_trait)]
impl Actor for TestActor {
    type Msg = TestMessage;
    type Arguments = ();
    type State = ();

    async fn pre_start(
        &self,
        _this_actor: ActorRef<Self::Msg>,
        _: (),
    ) -> Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        _: ActorRef<Self::Msg>,
        message: Self::Msg,
        _: &mut Self::State,
    ) -> Result<(), ActorProcessingErr> {
        match message {
            TestMessage::Add(a, b, reply) => {
                let _ = reply.send(a + b);
            }
            TestMessage::Echo(s, reply) => {
                let _ = reply.send(s);
            }
            TestMessage::CastOnly(_) => {}
        }
        Ok(())
    }
}

async fn spawn_test_actor() -> (
    ActorRef<TestMessage>,
    impl std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>,
) {
    let (actor, handle) = Actor::spawn(None, TestActor, ())
        .await
        .expect("Actor spawn failed");
    let wrapped_handle = async move {
        handle
            .await
            .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    };
    (actor, wrapped_handle)
}

#[crate::concurrency::test]
async fn test_call_macro() {
    let (actor, handle) = spawn_test_actor().await;

    let result = call!(actor, TestMessage::Add, 2, 3);
    assert!(result.is_ok(), "call! macro failed: {:?}", result.err());
    assert_eq!(result.unwrap(), 5);

    actor.stop(None);
    assert!(handle.await.is_ok(), "Actor handle join failed");
}

#[crate::concurrency::test]
async fn test_call_macro_echo() {
    let (actor, handle) = spawn_test_actor().await;

    let msg = "hello".to_string();
    let result = call!(actor, TestMessage::Echo, msg.clone());
    assert!(result.is_ok(), "call! macro failed: {:?}", result.err());
    assert_eq!(result.unwrap(), msg);

    actor.stop(None);
    assert!(handle.await.is_ok(), "Actor handle join failed");
}

#[crate::concurrency::test]
async fn test_call_t_macro_success() {
    let (actor, handle) = spawn_test_actor().await;

    let result = call_t!(actor, TestMessage::Add, 100, 4, 6);
    assert!(result.is_ok(), "call_t! macro failed: {:?}", result.err());
    assert_eq!(result.unwrap(), 10);

    actor.stop(None);
    assert!(handle.await.is_ok(), "Actor handle join failed");
}

#[crate::concurrency::test]
async fn test_cast_macro() {
    let (actor, handle) = spawn_test_actor().await;

    let res = cast!(actor, TestMessage::CastOnly(42));
    assert!(res.is_ok(), "cast! macro failed: {:?}", res.err());

    actor.stop(None);
    assert!(handle.await.is_ok(), "Actor handle join failed");
}

// TODO: forward! macro is not tested here due to its complexity and need for two actors.
