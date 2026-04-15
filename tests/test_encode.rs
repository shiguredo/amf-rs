use shiguredo_amf::ReconfigureParams;

/// ReconfigureParams::default() が全項目未指定であることを確認する
#[test]
fn test_reconfigure_params_default_is_empty() {
    let params = ReconfigureParams::default();
    assert_eq!(params.framerate_num, None);
    assert_eq!(params.framerate_den, None);
    assert_eq!(params.target_kbps, None);
    assert_eq!(params.max_kbps, None);
    assert_eq!(params.qpi, None);
    assert_eq!(params.qpp, None);
    assert_eq!(params.qpb, None);
    assert_eq!(params.gop_pic_size, None);
}
