mod integration {
    mod cli_exit_test;
    mod dashboard_test;
    mod datagraph_test;
    #[cfg(feature = "metacall-deploy")]
    mod deploy_client_call_test;
    #[cfg(feature = "metacall-deploy")]
    mod deploy_edge_cases_test;
    #[cfg(feature = "metacall-deploy")]
    mod deploy_mixed_test;
    #[cfg(feature = "metacall-deploy")]
    mod deploy_test;
    mod edge_cases_test;
    mod graph_regression_test;
    mod import_test;
    mod inspect_output_test;
    mod output_defaults_test;
    mod output_format_test;
    mod pipeline_equivalence_test;
    mod pipeline_test;
    mod reanalyze_api_test;
    mod shard_hygiene_test;
    mod shard_index_test;
    mod shard_test;
    #[cfg(feature = "watch")]
    mod watch_test;
}
