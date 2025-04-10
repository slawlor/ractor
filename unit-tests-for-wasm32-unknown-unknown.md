# Steps to run unit tests for target `wasm32-unknown-unknown`

`ractor` is supposed to support wasm32-unknown-unknown in browser, but unit-tests can't be executed in CI due to some known issues (see https://github.com/rustwasm/wasm-pack/issues/1424 ). Here are steps to run unit-tests manually.

## Install dependencies

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

`wasm-pack` is a bundler and test executor for wasm targets.

## Run unit tests in browser

### Build unit-tests and start web server

```sh
wasm-pack test --firefox ./ractor
```
### Wait for unit-tests to be built and server started

```console
Interactive browsers tests are now available at http://127.0.0.1:8000

Note that interactive mode is enabled because `NO_HEADLESS`
is specified in the environment of this process. Once you're
done with testing you'll need to kill this server with
Ctrl-C.
```

Now access `http://127.0.0.1:8000` in browser.

### Monitor test progress in browser

```console
running 84 tests
test factory::queues::tests::test_priority_queueing ... ok
test factory::queues::tests::test_basic_queueing ... ok
test factory::tests::dynamic_discarding::test_dynamic_dispatch_basic ... ok
test port::output::tests::output_port_subscriber_tests::test_output_port_subscriber ... ok
test tests::test_platform_sleep_works ... ok
test tests::test_error_message_extraction ... ok
test factory::ratelim::tests::test_leaky_bucket_max ... ok
test factory::ratelim::tests::test_basic_leaky_bucket ... ok
test time::tests::test_kill_after ... ok
test time::tests::test_exit_after ... ok
test time::tests::test_send_after ... ok
test time::tests::test_intervals ... ok
test actor::tests::supervisor::draining_children_will_shutdown_parent_too ... ok
test actor::tests::supervisor::draining_children_and_wait_during_parent_shutdown ... ok
test actor::tests::supervisor::stopping_children_will_shutdown_parent_too ... ok
test actor::tests::supervisor::stopping_children_and_wait_during_parent_shutdown ... ok
test actor::tests::supervisor::test_supervisor_double_link ... ok
test actor::tests::supervisor::test_supervisor_captures_dead_childs_state ... ok
test actor::tests::supervisor::instant_supervised_spawns ... ok
test actor::tests::supervisor::test_killing_a_supervisor_terminates_children ... ok
test actor::tests::supervisor::test_supervision_error_in_supervisor_handle ... ok
test actor::tests::supervisor::test_supervision_error_in_post_stop ... ok
test actor::tests::supervisor::test_supervision_error_in_handle ... ok
test actor::tests::supervisor::test_supervision_error_in_post_startup ... ok
test factory::tests::dynamic_settings::test_dynamic_settings ... ok
test pg::tests::test_scope_monitoring ... ok
test pg::tests::test_pg_monitoring ... ok
test pg::tests::test_actor_leaves_scope_manually ... ok
test pg::tests::test_actor_leaves_pg_group_manually ... ok
test pg::tests::test_actor_leaves_scope_on_shupdown ... ok
test pg::tests::test_actor_leaves_pg_group_on_shutdown ... ok
test pg::tests::test_multiple_groups_in_multiple_scopes ... ok
test pg::tests::test_multiple_groups ... ok
test pg::tests::test_which_scoped_groups ... ok
test pg::tests::test_multiple_members_in_scoped_group ... ok
test pg::tests::test_multiple_members_in_group ... ok
test pg::tests::test_which_scopes_and_groups ... ok
test pg::tests::test_basic_group_in_named_scope ... ok
test pg::tests::test_basic_group_in_default_scope ... ok
test factory::tests::lifecycle::test_lifecycle_hooks ... ok
test factory::tests::ratelim::test_leaky_bucket_rate_limiting ... ok
test factory::tests::ratelim::test_factory_rate_limiting_custom_hash ... ok
test factory::tests::ratelim::test_factory_rate_limiting_round_robin ... ok
test factory::tests::ratelim::test_factory_rate_limiting_key_persistent ... ok
test factory::tests::ratelim::test_factory_rate_limiting_sticky_queuer ... ok
test factory::tests::ratelim::test_factory_rate_limiting_queuer ... ok
test registry::tests::test_actor_registry_unenrollment ... ok
test registry::tests::test_duplicate_registration ... ok
test registry::tests::test_basic_registation ... ok
test factory::tests::draining_requests::test_request_draining ... ok
test rpc::tests::test_multi_call ... ok
test rpc::tests::test_rpc_call_forwarding ... ok
test rpc::tests::test_rpc_call ... ok
test rpc::tests::test_rpc_cast ... ok
test port::output::tests::test_delivery ... ok
test port::output::tests::test_50_receivers ... ok
test port::output::tests::test_single_forward ... ok
test factory::tests::priority_queueing::test_basic_priority_queueing ... ok
test factory::tests::basic::test_discarding_new_records_on_queuer ... ok
test factory::tests::basic::test_stuck_workers ... ok
test factory::tests::basic::test_discarding_old_records_on_queuer ... ok
test factory::tests::basic::test_dispatch_sticky_queueing ... ok
test factory::tests::basic::test_dispatch_custom_hashing ... ok
test factory::tests::basic::test_dispatch_round_robin ... ok
test factory::tests::basic::test_dispatch_queuer ... ok
test factory::tests::basic::test_dispatch_key_persistent ... ok
test actor::tests::derived_actor_ref ... ok
test actor::tests::wait_for_death ... ok
test actor::tests::runtime_message_typing ... ok
test actor::tests::actor_drain_messages ... ok
test actor::tests::actor_post_stop_executed_before_stop_and_wait_returns ... ok
test actor::tests::actor_failing_in_spawn_err_doesnt_poison_registries ... ok
test actor::tests::kill_and_wait ... ok
test actor::tests::stop_and_wait ... ok
test actor::tests::instant_spawns ... ok
test actor::tests::test_sending_message_to_dead_actor ... ok
test actor::tests::test_sending_message_to_invalid_actor_type ... ok
test actor::tests::test_kill_terminates_supervision_work ... ok
test actor::tests::test_stop_does_not_terminate_async_work ... ok
test actor::tests::test_kill_terminates_work ... ok
test actor::tests::test_stop_higher_priority_over_messages ... ok
test actor::tests::test_error_on_start_captured ... ok
test factory::tests::dynamic_pool::test_worker_pool_adjustment_automatic ... ok
test factory::tests::dynamic_pool::test_worker_pool_adjustment_manual ... ok

test result: ok. 84 passed; 0 failed; 0 ignored; 0 filtered out; finished in 165.20s
```

### Clean up

Press `Ctrl + C` to shutdown the web server started by `wasm-pack`
