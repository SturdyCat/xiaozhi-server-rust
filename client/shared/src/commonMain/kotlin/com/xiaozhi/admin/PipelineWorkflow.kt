package com.xiaozhi.admin

/** A core stage or a selectable attachment point; an empty point is not a fake node. */
data class WorkflowPosition(
    val id: String,
    val label: String,
    val nodes: List<PipelineFlowNode>,
    val stage: VoiceStageId? = null,
) {
    val isCore: Boolean get() = stage != null
}

/** Horizontal projection of the existing flow, not another configuration/test registry. */
object PipelineWorkflow {
    val positions: List<WorkflowPosition> = build(PipelineFlow.all)

    fun forId(id: String): WorkflowPosition? = positions.firstOrNull { it.id == id }

    fun forNode(nodeId: String): WorkflowPosition? =
        positions.firstOrNull { position -> position.nodes.any { it.id == nodeId } }

    /** Legacy two-field node routes still work. Bare sections remain outside the workflow. */
    fun resolve(selection: ConfigSelection): WorkflowPosition? {
        val nodeId = selection.nodeId
        val explicit = selection.workflowPositionId
        if (nodeId != null) {
            val node = PipelineFlow.forId(nodeId) ?: return null
            if (node.sectionId != selection.sectionId) return null
            val position = forNode(nodeId) ?: return null
            return position.takeIf { explicit == null || explicit == it.id }
        }
        // Only genuinely empty attachment points use the synthetic *route* section.
        // No section descriptor, node, configuration, test, or capability is fabricated.
        val position = explicit?.let(::forId) ?: return null
        return position.takeIf { selection.sectionId == "workflow" && it.nodes.isEmpty() }
    }

    fun selectionForPosition(id: String): ConfigSelection? = forId(id)?.let { position ->
        position.nodes.firstOrNull()?.let { node ->
            ConfigSelection(node.sectionId, node.id, position.id)
        } ?: ConfigSelection("workflow", null, position.id)
    }

    /** Every attachment anchor has exactly one geometric home, including future features. */
    internal fun build(nodes: List<PipelineFlowNode>): List<WorkflowPosition> {
        require(nodes.map { it.id }.distinct().size == nodes.size) { "Flow node IDs must be unique" }
        val stages = VoiceStageId.entries
        fun attached(vararg anchors: PipelineAnchor) = nodes.filter { it.anchor in anchors }
        fun stageLabel(stage: VoiceStageId) = stage.id.uppercase()
        val result = buildList {
            add(WorkflowPosition("start", "开始", attached(PipelineAnchor.Start, PipelineAnchor.Before(stages.first()))))
            stages.forEachIndexed { index, stage ->
                val core = nodes.filter { it.coreStage == stage }
                require(core.size == 1 && core.single().anchor == null) { "Each stage needs one unanchored core node" }
                add(WorkflowPosition(stage.id, stageLabel(stage), core, stage))
                val next = stages.getOrNull(index + 1)
                if (next != null) {
                    add(WorkflowPosition(
                        "link.${stage.id}.${next.id}", "${stageLabel(stage)} → ${stageLabel(next)}",
                        attached(PipelineAnchor.After(stage), PipelineAnchor.Before(next)),
                    ))
                }
            }
            add(WorkflowPosition("end", "结束", attached(PipelineAnchor.After(stages.last()), PipelineAnchor.End)))
        }
        val assigned = result.flatMap { it.nodes }
        require(assigned.size == nodes.size && assigned.map { it.id }.toSet() == nodes.map { it.id }.toSet()) {
            "Every flow node must belong to exactly one workflow position"
        }
        return result
    }
}
