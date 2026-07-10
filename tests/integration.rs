mod integration {
    mod dashboard_test;
    #[cfg(feature = "metacall-deploy")]
    mod deploy_edge_cases_test;
    #[cfg(feature = "metacall-deploy")]
    mod deploy_test;
    mod edge_cases_test;
    mod inspect_output_test;
    mod output_format_test;
    mod pipeline_test;
}
