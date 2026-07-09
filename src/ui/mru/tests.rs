use proptest::prelude::*;
use proptest_derive::Arbitrary;

use super::*;

fn create_thumbnail() -> Thumbnail {
    Thumbnail {
        id: MappedId::next(),
        timestamp: None,
        on_current_output: false,
        on_current_workspace: false,
        app_id: None,
        size: Size::new(100, 100),
        clock: Clock::with_time(Duration::ZERO),
        config: niri_config::MruPreviews::default(),
        open_animation: None,
        move_animation: None,
        title_texture: Default::default(),
        background: RefCell::new(FocusRing::new(Default::default())),
        border: RefCell::new(FocusRing::new(Default::default())),
    }
}

#[test]
fn remove_last_window_out_of_two() {
    let ops = [Op::Backward, Op::Remove(1)];

    let thumbnails = vec![create_thumbnail(), create_thumbnail()];
    let current_id = thumbnails.first().map(|t| t.id);
    let mut mru = WindowMru {
        thumbnails,
        current_id,
        scope: MruScope::All,
        app_id_filter: None,
    };

    check_ops(&mut mru, &ops);
}

fn arbitrary_scope() -> impl Strategy<Value = MruScope> {
    prop_oneof![
        Just(MruScope::All),
        Just(MruScope::Output),
        Just(MruScope::Workspace),
    ]
}

fn arbitrary_filter() -> impl Strategy<Value = MruFilter> {
    prop_oneof![Just(MruFilter::All), Just(MruFilter::AppId)]
}

fn arbitrary_app_id() -> impl Strategy<Value = Option<String>> {
    prop_oneof![Just(None), Just(Some(1)), Just(Some(2))]
        .prop_map(|id| id.map(|id| format!("app-{id}")))
}

prop_compose! {
    fn arbitrary_thumbnail()(
        timestamp: Option<Duration>,
        on_current_output: bool,
        on_current_workspace: bool,
        app_id in arbitrary_app_id(),
    ) -> Thumbnail {
        let mut thumbnail = create_thumbnail();
        thumbnail.timestamp = timestamp;
        thumbnail.on_current_workspace = on_current_workspace;
        thumbnail.on_current_output = on_current_output;
        thumbnail.app_id = app_id;
        thumbnail
    }
}

prop_compose! {
    fn arbitrary_mru()(
        thumbnails in proptest::collection::vec(arbitrary_thumbnail(), 1..10),
    ) -> WindowMru {
        let current_id = thumbnails.first().map(|t| t.id);
        WindowMru {
            thumbnails,
            current_id,
            scope: MruScope::All,
            app_id_filter: None,
        }
    }
}

#[derive(Debug, Clone, Arbitrary)]
enum Op {
    Forward,
    Backward,
    First,
    Last,
    SetScope(#[proptest(strategy = "arbitrary_scope()")] MruScope),
    SetFilter(#[proptest(strategy = "arbitrary_filter()")] MruFilter),
    Remove(#[proptest(strategy = "1..10usize")] usize),
}

impl Op {
    fn apply(&self, mru: &mut WindowMru) {
        match self {
            Op::Forward => mru.forward(),
            Op::Backward => mru.backward(),
            Op::First => mru.first(),
            Op::Last => mru.last(),
            Op::SetScope(scope) => {
                mru.set_scope(*scope);
            }
            Op::SetFilter(filter) => {
                mru.set_filter(*filter);
            }
            Op::Remove(idx) => {
                if *idx < mru.thumbnails.len() {
                    mru.remove_by_idx(*idx);
                }
            }
        }
    }
}

fn check_ops(mru: &mut WindowMru, ops: &[Op]) {
    for op in ops {
        op.apply(mru);
        mru.verify_invariants();
    }
}

proptest! {
    #[test]
    fn random_operations_dont_panic(
        mut mru in arbitrary_mru(),
        ops: Vec<Op>,
    ) {
        check_ops(&mut mru, &ops);
    }
}
