//! task_exec 测试（仅测试编译）——重试续跑（B 案上下文接续）：
//! a) retry() 把 session_id 抢救进 last_session_id（内存端口层）；
//! b) prompt 接续块纯函数：有输出含指令头+尾部、超长截断、空输出不产生接续块。

use super::*;
use super::test_util::*;

    #[tokio::test]
    async fn retry_rescues_session_id_into_last_session() {
        // 续跑命脉：reset_for_retry 会把 session_id 置 NULL——retry 必须先把旧会话 id
        // 抢救进 last_session_id，spawn 才能按它读上次输出尾部注入接续块。
        // manual 信任：重试入队后停在 plan 关（不 spawn），可确定性断言落库字段。
        let task_repo = Arc::new(InMemoryTaskStore::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            Arc::new(InMemoryApprovalStore::new()),
            sessions,
            maps,
            harness_stub("框架"),
            4,
            Arc::new(FixedAgentSlot::new(Arc::new("true".into()), Arc::new(vec![]))),
            None,
        );
        let mut task = sample_task("interrupted");
        task.session_id = Some("sess-dead-1".into());
        seed_task(task_repo.as_ref(), &task);
        executor.retry("task-t1").await.unwrap();
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.last_session_id.as_deref(), Some("sess-dead-1"), "旧会话 id 必须被抢救进 last_session_id");
        assert_eq!(t.session_id, None, "reset_for_retry 清 session_id 的语义不变");
        // retry 内部同步走 enqueue_pending → execute：manual 任务已被推到 plan 审批关
        // （awaiting_approval）——重试链路照常运转，本测试只锁 last_session_id 抢救语义。
        assert_eq!(t.status, "awaiting_approval");

        // 幂等回退：无会话 id 的任务重试，last_session_id 保持 NULL（不凭空产生接续线索）
        let mut task2 = sample_task("failed");
        task2.id = "task-t2".into();
        task2.session_id = None;
        seed_task(task_repo.as_ref(), &task2);
        executor.retry("task-t2").await.unwrap();
        let t2 = task_repo.get("task-t2").await.unwrap().unwrap();
        assert_eq!(t2.last_session_id, None, "无旧会话不得写 last_session_id（空输出幂等回退）");
    }

    #[test]
    fn resume_block_wraps_tail_with_instruction_header() {
        let block = resume_block("已改完 module/a.rs\n正在写 module/b.rs");
        assert!(block.contains("【上次执行进展（会话因中断停止，请从中断点继续，不要重做已完成工作）】"), "必须带固定指令头");
        assert!(block.contains("已改完 module/a.rs"), "尾部原样注入");
        assert!(block.contains("正在写 module/b.rs"), "尾部末行保留");
        assert!(block.starts_with("\n\n"), "接续块与 prompt 主体分隔");
    }

    #[test]
    fn resume_block_truncates_to_shorter_of_lines_and_chars() {
        // 行数截断：300 行只留最后 150 行
        let many: String = (1..=300).map(|i| format!("line-{i}")).collect::<Vec<_>>().join("\n");
        let block = resume_block(&many);
        assert!(!block.contains("line-1\n"), "头部行必须被截掉");
        assert!(block.contains("line-151"), "恰留最后 150 行（首行 = 总 300 - 150 + 1）");
        assert!(block.contains("line-300"), "尾部末行保留");
        // 字符截断：超长单行保尾丢头
        let long = "x".repeat(RESUME_TAIL_CHARS * 2);
        let block2 = resume_block(&long);
        assert_eq!(block2.matches('x').count(), RESUME_TAIL_CHARS, "字符截断：只留尾部 8k 字符（保尾丢头）");
        assert!(block2.trim_end().ends_with('x'), "尾部 x 保留");
    }

    #[test]
    fn resume_block_empty_output_falls_back_to_plain_retry() {
        assert_eq!(resume_block(""), "", "空输出 = 无接续块（原样重试）");
        assert_eq!(resume_block("   \n  \n "), "", "纯空白输出 = 无接续块");
        assert_eq!(resume_block("\n\n"), "", "仅换行 = 无接续块");
    }

    #[test]
    fn tail_of_output_keeps_last_lines_and_chars() {
        // 行数上限内不动；超行数取尾；超字符保尾丢头
        assert_eq!(tail_of_output("a\nb\nc", 10, 100), "a\nb\nc");
        assert_eq!(tail_of_output("a\nb\nc", 2, 100), "b\nc");
        let long = "y".repeat(100);
        let t = tail_of_output(&long, 10, 50);
        assert_eq!(t.chars().count(), 50);
        assert!(t.chars().all(|c| c == 'y'));
    }
